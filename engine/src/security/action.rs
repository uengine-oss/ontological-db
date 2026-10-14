//! Actions — named, parameterised writes (spec 014 FR-011..FR-016).
//!
//! The Palantir model, built out of PostgreSQL parts. An action is:
//!
//!   * a body — one Cypher (or TypeQL) statement whose only inputs are the
//!     bound `$params`, so a caller supplies values and never text;
//!   * a parameter contract — required keys and their JSON types, and nothing
//!     else is accepted;
//!   * a precondition — a read query that must return `ok = true` for these
//!     parameters (the "submission criteria");
//!   * `run_as` — the role whose write grants the body runs with.
//!
//! Each action gets its own function `og_action.og_action_<id>(jsonb)`, SECURITY
//! DEFINER and owned by `run_as`. Whether a caller may run the action is then
//! PostgreSQL's EXECUTE check on that function, made against the caller; what
//! the action may touch is whatever `run_as` was granted. An agent that holds
//! read access and EXECUTE on three actions can do those three things, however
//! its Cypher is phrased, because it has no other way to write.

use crate::catalog::types;
use crate::spiu;
use pgrx::prelude::*;
use pgrx::JsonB;
use pgrx::PgSqlErrorCode;
use serde_json::Value;

/// Define (or redefine) an action. Admin only.
///
/// The caller must be able to `SET ROLE run_as` — PostgreSQL requires it to
/// hand the action function to that role.
#[pg_extern]
#[allow(clippy::too_many_arguments)]
fn og_define_action(
    graph: &str,
    name: &str,
    body: &str,
    run_as: &str,
    lang: default!(&str, "'cypher'"),
    params: default!(JsonB, "'{}'"),
    precondition: default!(Option<&str>, "NULL"),
    description: default!(Option<&str>, "NULL"),
) -> i32 {
    let gid = types::graph_id(graph);
    if spiu::one::<i32>("SELECT 1 FROM pg_roles WHERE rolname = $1", &[run_as.into()])
        .ok()
        .flatten()
        .is_none()
    {
        error!("role '{run_as}' does not exist");
    }
    if !name.chars().all(|c| c.is_alphanumeric() || c == '_') || name.is_empty() {
        error!("action name '{name}' must be letters, digits and '_'");
    }
    match lang {
        "cypher" => {
            if let Err(e) = crate::cypher::parser::parse(body) {
                error!("action body does not parse: {e}");
            }
        }
        "typeql" => {}
        other => error!("unknown action language '{other}' (cypher | typeql)"),
    }
    if let Some(pre) = precondition {
        if let Err(e) = crate::cypher::parser::parse(pre) {
            error!("precondition does not parse: {e}");
        }
    }
    let Some(spec) = params.0.as_object() else {
        error!("params must be an object of name → type, e.g. {{\"name\": \"string\"}}");
    };
    for (k, v) in spec {
        let ok = matches!(v.as_str(), Some("string" | "int" | "float" | "number" | "bool" | "any" | "list" | "map"));
        if !ok {
            error!("parameter '{k}': type must be one of string, int, float, number, bool, list, map, any");
        }
    }

    // Replace an existing definition in place: drop its function first.
    if let Some(old) = spiu::one::<String>(
        "SELECT fn_name FROM og_catalog.action WHERE graph_id = $1 AND name = $2",
        &[gid.into(), name.into()],
    )
    .ok()
    .flatten()
    {
        Spi::run(&format!("DROP FUNCTION IF EXISTS og_action.{}(jsonb)", spiu::ident(&old))).ok();
        Spi::run_with_args(
            "DELETE FROM og_catalog.action WHERE graph_id = $1 AND name = $2",
            &[gid.into(), name.into()],
        )
        .ok();
    }

    let id = spiu::one_mut::<i32>(
        "INSERT INTO og_catalog.action
             (graph_id, name, lang, body, params, precondition, run_as, fn_name, description)
         VALUES ($1, $2, $3, $4, $5, $6, $7, '', $8) RETURNING action_id",
        &[
            gid.into(),
            name.into(),
            lang.into(),
            body.into(),
            params.into(),
            precondition.into(),
            run_as.into(),
            description.into(),
        ],
    )
    .expect("action insert failed")
    .unwrap();
    let fn_name = format!("og_action_{id}");
    Spi::run_with_args(
        "UPDATE og_catalog.action SET fn_name = $2 WHERE action_id = $1",
        &[id.into(), fn_name.clone().into()],
    )
    .expect("action update failed");

    let schema = spiu::ident(&ext_schema());
    let f = format!("og_action.{}", spiu::ident(&fn_name));
    let owner = spiu::ident(run_as);
    Spi::run(&format!(
        "CREATE FUNCTION {f}(params jsonb DEFAULT '{{}}'::jsonb) RETURNS SETOF jsonb
         LANGUAGE sql SECURITY DEFINER SET search_path = {schema}, pg_catalog, pg_temp
         AS $body$ SELECT og__action_exec({id}, params) $body$"
    ))
    .unwrap_or_else(|e| error!("could not create the action function: {e}"));
    Spi::run(&format!("REVOKE ALL ON FUNCTION {f}(jsonb) FROM PUBLIC"))
        .unwrap_or_else(|e| error!("revoke failed: {e}"));
    // The executor is reachable only through an action function, i.e. only by
    // an action's owner.
    Spi::run(&format!(
        "GRANT EXECUTE ON FUNCTION {schema}.og__action_exec(int4, jsonb) TO {owner}"
    ))
    .unwrap_or_else(|e| error!("grant failed: {e}"));
    Spi::run(&format!("GRANT CREATE, USAGE ON SCHEMA og_action TO {owner}"))
        .unwrap_or_else(|e| error!("grant failed: {e}"));
    Spi::run(&format!("ALTER FUNCTION {f}(jsonb) OWNER TO {owner}")).unwrap_or_else(|e| {
        error!("could not hand the action to '{run_as}' (you must be able to SET ROLE to it): {e}")
    });
    id
}

/// Let a role run an action.
#[pg_extern]
fn og_grant_action(role: &str, graph: &str, name: &str) {
    let f = action_fn(graph, name);
    let r = spiu::ident(role);
    Spi::run(&format!("GRANT USAGE ON SCHEMA og_action TO {r}"))
        .unwrap_or_else(|e| error!("grant failed: {e}"));
    Spi::run(&format!("GRANT EXECUTE ON FUNCTION og_action.{}(jsonb) TO {r}", spiu::ident(&f)))
        .unwrap_or_else(|e| error!("grant failed: {e}"));
}

#[pg_extern]
fn og_revoke_action(role: &str, graph: &str, name: &str) {
    let f = action_fn(graph, name);
    Spi::run(&format!(
        "REVOKE EXECUTE ON FUNCTION og_action.{}(jsonb) FROM {}",
        spiu::ident(&f),
        spiu::ident(role)
    ))
    .unwrap_or_else(|e| error!("revoke failed: {e}"));
}

#[pg_extern]
fn og_drop_action(graph: &str, name: &str) {
    let f = action_fn(graph, name);
    Spi::run(&format!("DROP FUNCTION IF EXISTS og_action.{}(jsonb)", spiu::ident(&f)))
        .unwrap_or_else(|e| error!("drop failed: {e}"));
    Spi::run_with_args(
        "DELETE FROM og_catalog.action WHERE graph_id = $1 AND name = $2",
        &[types::graph_id(graph).into(), name.into()],
    )
    .unwrap_or_else(|e| error!("delete failed: {e}"));
}

fn action_fn(graph: &str, name: &str) -> String {
    let gid = types::graph_id(graph);
    spiu::one::<String>(
        "SELECT fn_name FROM og_catalog.action WHERE graph_id = $1 AND name = $2",
        &[gid.into(), name.into()],
    )
    .ok()
    .flatten()
    .unwrap_or_else(|| error!("graph '{graph}' has no action named '{name}'"))
}

/// Run an action. The EXECUTE check on the action's function is the gate.
#[pg_extern]
fn og_run_action(
    graph: &str,
    name: &str,
    params: default!(JsonB, "'{}'"),
) -> SetOfIterator<'static, JsonB> {
    let f = action_fn(graph, name);
    let rows: Vec<JsonB> = Spi::connect_mut(|client| {
        client
            .update(
                &format!("SELECT r FROM og_action.{}($1) AS r", spiu::ident(&f)),
                None,
                &[params.into()],
            )
            .map(|t| t.filter_map(|r| r.get::<JsonB>(1).ok().flatten()).collect())
            .unwrap_or_else(|e| error!("{e}"))
    });
    SetOfIterator::new(rows)
}

/// The body of every action function. Runs as the action's owner.
#[allow(non_snake_case)]
#[pg_extern]
fn og__action_exec(action_id: i32, params: JsonB) -> SetOfIterator<'static, JsonB> {
    let row = Spi::connect(|client| {
        let mut t = client
            .select(
                "SELECT g.name, a.name, a.lang, a.body, a.params, a.precondition
                   FROM og_catalog.action a JOIN og_catalog.graph g USING (graph_id)
                  WHERE a.action_id = $1",
                Some(1),
                &[action_id.into()],
            )
            .ok()?;
        let r = t.next()?;
        Some((
            r.get::<String>(1).ok()??,
            r.get::<String>(2).ok()??,
            r.get::<String>(3).ok()??,
            r.get::<String>(4).ok()??,
            r.get::<JsonB>(5).ok()??.0,
            r.get::<String>(6).ok()?,
        ))
    });
    let Some((graph, name, lang, body, spec, pre)) = row else {
        error!("action {action_id} does not exist");
    };
    check_params(&name, &spec, &params.0);

    // Restored on every way out, including an error: PostgreSQL errors unwind
    // through here as Rust panics, and a guard's Drop runs on unwind. Without
    // it a failed action left its name on every later audit row of the backend.
    let _guard = ActionGuard(super::CURRENT_ACTION.with(|a| a.replace(Some(name.clone()))));
    if let Some(pre) = pre {
        let rows = crate::cypher::run_query(&graph, &pre, &params.0);
        let ok = rows
            .first()
            .and_then(|r| r.as_object())
            .and_then(|o| o.get("ok").or_else(|| o.values().next()))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !ok {
            ereport!(
                ERROR,
                PgSqlErrorCode::ERRCODE_CHECK_VIOLATION,
                format!("OG_ACTION_REJECTED: the precondition of action '{name}' did not hold for these parameters")
            );
        }
    }
    let out = match lang.as_str() {
        "typeql" => crate::typeql::run_action_body(&graph, &body),
        _ => crate::cypher::run_query(&graph, &body, &params.0),
    };
    SetOfIterator::new(out.into_iter().map(JsonB))
}

struct ActionGuard(Option<String>);

impl Drop for ActionGuard {
    fn drop(&mut self) {
        let prev = self.0.take();
        super::CURRENT_ACTION.with(|a| *a.borrow_mut() = prev);
    }
}

fn check_params(name: &str, spec: &Value, given: &Value) {
    let Some(given) = given.as_object() else {
        error!("action '{name}': parameters must be a JSON object");
    };
    let spec = spec.as_object().cloned().unwrap_or_default();
    for k in given.keys() {
        if !spec.contains_key(k) {
            ereport!(
                ERROR,
                PgSqlErrorCode::ERRCODE_INVALID_PARAMETER_VALUE,
                format!("action '{name}' does not take a parameter named '{k}'")
            );
        }
    }
    for (k, ty) in &spec {
        let Some(v) = given.get(k) else {
            ereport!(
                ERROR,
                PgSqlErrorCode::ERRCODE_INVALID_PARAMETER_VALUE,
                format!("action '{name}' requires parameter '{k}'")
            );
        };
        let ok = match ty.as_str().unwrap_or("any") {
            "string" => v.is_string(),
            "int" => v.is_i64() || v.is_u64(),
            "float" | "number" => v.is_number(),
            "bool" => v.is_boolean(),
            "list" => v.is_array(),
            "map" => v.is_object(),
            _ => true,
        };
        if !ok {
            ereport!(
                ERROR,
                PgSqlErrorCode::ERRCODE_INVALID_PARAMETER_VALUE,
                format!("action '{name}': parameter '{k}' must be a {}", ty.as_str().unwrap_or("value"))
            );
        }
    }
}

fn ext_schema() -> String {
    spiu::one::<String>(
        "SELECT n.nspname FROM pg_extension e
           JOIN pg_namespace n ON n.oid = e.extnamespace
          WHERE e.extname = 'ontological'",
        &[],
    )
    .ok()
    .flatten()
    .unwrap_or_else(|| "public".into())
}
