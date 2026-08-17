//! Graph storage — spec 001.
//!
//! Write paths live here in Rust because they must keep three structures in
//! lock-step inside one transaction (spec 001 FR-012): the registry, the typed
//! property table, and both directions of the adjacency segment.
//!
//! Read paths are deliberately **not** here. The Cypher compiler emits SQL that
//! touches `og_data.og_adj` directly so the PostgreSQL planner sees the whole
//! traversal — that is the difference between this design and a function-call
//! pipeline the optimiser cannot look into.

pub mod adjacency;
pub mod stats;

use crate::catalog::{labeling, types};
use crate::id;
use pgrx::prelude::*;
use pgrx::JsonB;
use serde_json::Value;

/// Allocate the next local id inside a type's 36-bit space and compose the
/// full identifier.
pub fn alloc_id(type_id: i32) -> i64 {
    let local = crate::spiu::one_mut::<i64>(
        "INSERT INTO og_data.og_id_alloc (type_id, next_id) VALUES ($1, 2)
         ON CONFLICT (type_id) DO UPDATE SET next_id = og_id_alloc.next_id + 1
         RETURNING next_id - 1",
        &[type_id.into()],
    )
    .expect("id allocation failed")
    .unwrap();
    id::make_id(0, type_id, local)
}

struct PropPlan {
    columns: Vec<String>,
    exprs: Vec<String>,
    declared: Vec<String>,
}

/// Build the column list / value expressions for a property payload.
///
/// Declared properties become real columns; everything else is funnelled into
/// `__ext`.  All values are extracted from ONE bound jsonb parameter, so no
/// user value is ever interpolated into SQL text (spec 003 FR-026).
/// The column type a JSON value wants.
///
/// Scalars only. An array or an object stays in the extension payload on
/// purpose: the one array property that matters here is `embedding`, and it is
/// declared as `vector(N)` by `og_add_embedding` when a vector index is
/// created. Declaring it as jsonb first would take that away.
fn infer_column_type(v: &Value) -> Option<&'static str> {
    match v {
        Value::Bool(_) => Some("bool"),
        Value::Number(n) => Some(if n.is_i64() || n.is_u64() { "int8" } else { "float8" }),
        Value::String(_) => Some("text"),
        _ => None,
    }
}

/// Column types this module is allowed to widen. Exactly the ones it can
/// create by inference — anything else was declared deliberately.
const WIDENABLE: &[&str] = &["bool", "int8", "float8"];

/// Can a column of `declared` hold a value that wants `wanted`?
fn type_accepts(declared: &str, wanted: &str) -> bool {
    if declared == wanted {
        return true;
    }
    // Widening within numbers is fine; everything else is not.
    matches!((declared, wanted), ("float8", "int8") | ("numeric", "int8") | ("numeric", "float8"))
}

/// Declare properties this type has not seen before, so they become real
/// columns instead of living in `__ext`.
///
/// Why this is worth doing at write time: a Cypher application never declares
/// anything — Neo4j has no schema to declare against. Without this every
/// property a Cypher app writes lands in the jsonb payload, where it cannot be
/// indexed, has no statistics, and (before this) lost its type on the way out.
///
/// A property whose value later disagrees with its column is **widened to
/// text** rather than refused. Neo4j allows a property to hold different types
/// on different nodes; text is the one column type that can represent all of
/// them, and widening is one-way so it cannot oscillate.
fn declare_new_props(type_id: i32, props: &Value, existing: &[(String, String, String)]) -> bool {
    let Some(obj) = props.as_object() else { return false };

    let Some((graph, type_name)) = Spi::connect(|client| -> Option<(String, String)> {
        let mut rows = client
            .select(
                "SELECT g.name, t.name FROM og_catalog.type t
                   JOIN og_catalog.graph g USING (graph_id) WHERE t.type_id = $1",
                Some(1),
                &[type_id.into()],
            )
            .ok()?;
        let r = rows.next()?;
        Some((r.get::<String>(1).ok()??, r.get::<String>(2).ok()??))
    }) else {
        return false;
    };

    let mut changed = false;
    for (key, value) in obj {
        let Some(want) = infer_column_type(value) else { continue };
        match existing.iter().find(|(n, _, _)| n == key) {
            None => {
                // `og_add_property` raises on a duplicate; this branch is the
                // only place that reaches it, and only for names not present.
                if Spi::run_with_args(
                    "SELECT og_add_property($1, $2, $3, $4, false, false)",
                    &[graph.clone().into(), type_name.clone().into(), key.as_str().into(), want.into()],
                )
                .is_ok()
                {
                    changed = true;
                }
            }
            // Widen only what we could have inferred ourselves. A property
            // declared as `vector(1536)` or `timestamptz` was declared on
            // purpose — by `og_add_embedding`, or by an application that knows
            // its own schema — and turning it into text because one write
            // disagreed would destroy that intent. (2026-08-16: doing so broke
            // the vector suite, which is what this guard is for.)
            Some((_, col, dtype))
                if WIDENABLE.contains(&dtype.as_str()) && !type_accepts(dtype, want) =>
            {
                let name = types::try_type_name(type_id);
                for sub in labeling::og_subtypes(type_id) {
                    if let Some(table) = types::storage_table(sub) {
                        // The type's alias view selects this column, and
                        // PostgreSQL will not alter a column a view depends on.
                        if let Some(n) = types::try_type_name(sub) {
                            types::drop_alias_view(&n);
                        }
                        let _ = Spi::run(&format!(
                            "ALTER TABLE {table} ALTER COLUMN {col} TYPE text USING {col}::text"
                        ));
                        if let Some(n) = types::try_type_name(sub) {
                            types::ensure_alias_view(sub, &n, &table);
                        }
                    }
                }
                let _ = name;
                let _ = Spi::run_with_args(
                    "UPDATE og_catalog.property SET data_type = 'text'
                      WHERE type_id = ANY($1) AND name = $2",
                    &[labeling::og_subtypes(type_id).into(), key.as_str().into()],
                );
                changed = true;
            }
            _ => {}
        }
    }
    changed
}

fn plan_props(type_id: i32, props: &Value, param: &str) -> PropPlan {
    let rows: Vec<(String, String, String)> = Spi::connect(|client| {
        client
            .select(
                "SELECT name, column_name, data_type FROM og_catalog.property WHERE type_id = $1",
                None,
                &[type_id.into()],
            )
            .expect("property lookup failed")
            .map(|r| {
                (
                    r.get::<String>(1).unwrap().unwrap(),
                    r.get::<String>(2).unwrap().unwrap(),
                    r.get::<String>(3).unwrap().unwrap(),
                )
            })
            .collect()
    });

    // Promote anything new to a real column before planning the write.
    let rows = if declare_new_props(type_id, props, &rows) {
        Spi::connect(|client| {
            client
                .select(
                    "SELECT name, column_name, data_type FROM og_catalog.property WHERE type_id = $1",
                    None,
                    &[type_id.into()],
                )
                .expect("property lookup failed")
                .map(|r| {
                    (
                        r.get::<String>(1).unwrap().unwrap(),
                        r.get::<String>(2).unwrap().unwrap(),
                        r.get::<String>(3).unwrap().unwrap(),
                    )
                })
                .collect()
        })
    } else {
        rows
    };

    let mut plan = PropPlan { columns: vec![], exprs: vec![], declared: vec![] };
    for (name, col, dtype) in rows {
        plan.declared.push(name.clone());
        if props.get(&name).is_none() {
            continue;
        }
        let lit = quote_json_key(&name);
        let expr = if dtype.ends_with("[]") {
            let elem = dtype.trim_end_matches("[]");
            format!(
                "(SELECT array_agg(x)::{dtype} FROM jsonb_array_elements_text({param}->{lit}) AS t(x_raw), \
                 LATERAL (SELECT t.x_raw::{elem}) AS c(x))"
            )
        } else {
            format!("({param}->>{lit})::{dtype}")
        };
        plan.columns.push(col);
        plan.exprs.push(expr);
    }
    plan
}

fn quote_json_key(k: &str) -> String {
    format!("'{}'", k.replace('\'', "''"))
}

fn ext_expr(plan: &PropPlan, param: &str) -> String {
    if plan.declared.is_empty() {
        format!("NULLIF({param}, '{{}}'::jsonb)")
    } else {
        let list = plan
            .declared
            .iter()
            .map(|d| format!("'{}'", d.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",");
        format!("NULLIF({param} - ARRAY[{list}]::text[], '{{}}'::jsonb)")
    }
}

// --------------------------------------------------------------------------
// Nodes
// --------------------------------------------------------------------------

#[pg_extern]
fn og_create_node(graph: &str, type_name: &str, props: default!(JsonB, "'{}'")) -> i64 {
    let gid = types::graph_id(graph);
    let tid = types::type_id(gid, type_name);
    create_node_inner(gid, tid, type_name, props.0)
}

pub fn create_node_inner(_gid: i32, tid: i32, type_name: &str, props: Value) -> i64 {
    if types::type_kind(tid) != 'e' {
        error!("'{type_name}' is not an entity type");
    }
    let table = types::storage_table(tid)
        .unwrap_or_else(|| error!("'{type_name}' is abstract and cannot be instantiated"));

    let nid = alloc_id(tid);
    let plan = plan_props(tid, &props, "$2");
    let ext = ext_expr(&plan, "$2");

    let mut cols = vec!["id".to_string()];
    let mut vals = vec!["$1".to_string()];
    cols.extend(plan.columns.iter().cloned());
    vals.extend(plan.exprs.iter().cloned());
    cols.push("__ext".into());
    vals.push(ext);

    Spi::run_with_args(
        "INSERT INTO og_data.og_node (id, type_id) VALUES ($1, $2)",
        &[nid.into(), tid.into()],
    )
    .expect("node registry insert failed");

    let sql = format!(
        "INSERT INTO {table} ({}) VALUES ({})",
        cols.join(", "),
        vals.join(", ")
    );
    // Counted before the value is moved into the statement, and reported only
    // once the statement has succeeded.
    let written = props.clone();
    Spi::run_with_args(&sql, &[nid.into(), JsonB(props).into()])
        .unwrap_or_else(|e| error!("node insert failed: {e}"));
    crate::stats::node_created();
    crate::stats::properties_set(&written);

    nid
}

#[pg_extern]
fn og_set_node_props(id: i64, props: JsonB) {
    set_node_props_inner(id, props.0);
}

pub fn set_node_props_inner(id: i64, props: Value) {
    let tid = id::id_type(id);
    let table = types::storage_table(tid).unwrap_or_else(|| error!("unknown type for id {id}"));
    let plan = plan_props(tid, &props, "$2");
    let written = props.clone();
    if plan.columns.is_empty() {
        let ext = ext_expr(&plan, "$2");
        Spi::run_with_args(
            &format!("UPDATE {table} SET __ext = COALESCE(__ext,'{{}}'::jsonb) || COALESCE({ext},'{{}}'::jsonb) WHERE id = $1"),
            &[id.into(), JsonB(props).into()],
        )
        .expect("update failed");
        crate::stats::properties_set(&written);
        return;
    }
    let sets: Vec<String> = plan
        .columns
        .iter()
        .zip(plan.exprs.iter())
        .map(|(c, e)| format!("{c} = {e}"))
        .collect();
    let ext = ext_expr(&plan, "$2");
    let sql = format!(
        "UPDATE {table} SET {}, __ext = COALESCE(__ext,'{{}}'::jsonb) || COALESCE({ext},'{{}}'::jsonb) WHERE id = $1",
        sets.join(", ")
    );
    Spi::run_with_args(&sql, &[id.into(), JsonB(props).into()]).expect("update failed");
    crate::stats::properties_set(&written);
}

/// Same, for edges.
pub fn set_edge_props_inner(id: i64, props: Value) {
    let tid = id::id_type(id);
    let table = types::storage_table(tid).unwrap_or_else(|| error!("unknown type for id {id}"));
    let plan = plan_props(tid, &props, "$2");
    let written = props.clone();
    let ext = ext_expr(&plan, "$2");
    let mut sets: Vec<String> = plan
        .columns
        .iter()
        .zip(plan.exprs.iter())
        .map(|(c, e)| format!("{c} = {e}"))
        .collect();
    sets.push(format!(
        "__ext = COALESCE(__ext,'{{}}'::jsonb) || COALESCE({ext},'{{}}'::jsonb)"
    ));
    let sql = format!("UPDATE {table} SET {} WHERE id = $1", sets.join(", "));
    Spi::run_with_args(&sql, &[id.into(), JsonB(props).into()]).expect("update failed");
    crate::stats::properties_set(&written);
}

/// Delete a node and every incident edge (DETACH DELETE semantics).
#[pg_extern]
fn og_delete_node(id: i64) -> i64 {
    delete_node_inner(id)
}

pub fn delete_node_inner(id: i64) -> i64 {
    let tid = id::id_type(id);
    let incident: Vec<i64> = Spi::connect(|client| {
        client
            .select(
                "SELECT DISTINCT e FROM og_data.og_adj a, LATERAL unnest(a.eid) AS e
                  WHERE a.src = $1",
                None,
                &[id.into()],
            )
            .expect("incident edge lookup failed")
            .filter_map(|r| r.get::<i64>(1).ok().flatten())
            .collect()
    });
    let mut removed = 0i64;
    for eid in incident {
        removed += delete_edge_inner(eid);
    }
    if let Some(table) = types::storage_table(tid) {
        Spi::run_with_args(&format!("DELETE FROM {table} WHERE id = $1"), &[id.into()])
            .expect("node delete failed");
    }
    Spi::run_with_args("DELETE FROM og_data.og_node WHERE id = $1", &[id.into()])
        .expect("registry delete failed");
    Spi::run_with_args("DELETE FROM og_data.og_adj WHERE src = $1", &[id.into()]).ok();
    // The incident edges counted themselves on the way through.
    crate::stats::node_deleted();
    removed + 1
}

// --------------------------------------------------------------------------
// Edges
// --------------------------------------------------------------------------

#[pg_extern]
fn og_create_edge(
    graph: &str,
    rel_type: &str,
    src: i64,
    dst: i64,
    props: default!(JsonB, "'{}'"),
) -> i64 {
    let gid = types::graph_id(graph);
    let tid = types::type_id(gid, rel_type);
    create_edge_inner(gid, tid, rel_type, src, dst, props.0)
}

pub fn create_edge_inner(
    _gid: i32,
    tid: i32,
    rel_type: &str,
    src: i64,
    dst: i64,
    props: Value,
) -> i64 {
    if types::type_kind(tid) != 'r' {
        error!("'{rel_type}' is not a relation type");
    }
    let table = types::storage_table(tid)
        .unwrap_or_else(|| error!("'{rel_type}' is abstract and cannot be instantiated"));

    validate_roles(tid, rel_type, src, dst);

    let eid = alloc_id(tid);
    let plan = plan_props(tid, &props, "$4");
    let ext = ext_expr(&plan, "$4");

    let mut cols = vec!["id".to_string(), "src".into(), "dst".into()];
    let mut vals = vec!["$1".to_string(), "$2".into(), "$3".into()];
    cols.extend(plan.columns.iter().cloned());
    vals.extend(plan.exprs.iter().cloned());
    cols.push("__ext".into());
    vals.push(ext);

    Spi::run_with_args(
        "INSERT INTO og_data.og_edge (id, type_id, src, dst) VALUES ($1, $2, $3, $4)",
        &[eid.into(), tid.into(), src.into(), dst.into()],
    )
    .expect("edge registry insert failed");

    let sql = format!(
        "INSERT INTO {table} ({}) VALUES ({})",
        cols.join(", "),
        vals.join(", ")
    );
    let written = props.clone();
    Spi::run_with_args(&sql, &[eid.into(), src.into(), dst.into(), JsonB(props).into()])
        .unwrap_or_else(|e| error!("edge insert failed: {e}"));

    // Both adjacency directions, same transaction — spec 001 FR-012.
    adjacency::append(src, tid, 'o', dst, eid);
    adjacency::append(dst, tid, 'i', src, eid);

    crate::stats::relationship_created();
    crate::stats::properties_set(&written);

    eid
}

/// Enforce role player types declared on the relation — spec 002 FR-017.
fn validate_roles(tid: i32, rel_type: &str, src: i64, dst: i64) {
    let roles: Vec<(String, i32, i32)> = Spi::connect(|client| {
        client
            .select(
                "SELECT name, ordinal, player_type_id FROM og_catalog.role
                  WHERE rel_type_id = ANY($1) AND player_type_id IS NOT NULL AND ordinal IN (0,1)",
                None,
                &[labeling::og_supertypes(tid).into()],
            )
            .expect("role lookup failed")
            .filter_map(|r| {
                Some((
                    r.get::<String>(1).ok()??,
                    r.get::<i32>(2).ok()??,
                    r.get::<i32>(3).ok()??,
                ))
            })
            .collect()
    });
    for (name, ordinal, player) in roles {
        let actual = if ordinal == 0 { id::id_type(src) } else { id::id_type(dst) };
        if !labeling::og_is_subtype(actual, player) {
            let expected = type_name_of(player);
            let got = type_name_of(actual);
            error!(
                "role '{name}' of relation '{rel_type}' requires a '{expected}', got '{got}'"
            );
        }
    }
}

pub fn type_name_of(tid: i32) -> String {
    crate::spiu::one::<String>(
        "SELECT name FROM og_catalog.type WHERE type_id = $1",
        &[tid.into()],
    )
    .ok()
    .flatten()
    .unwrap_or_else(|| format!("type#{tid}"))
}

#[pg_extern]
fn og_delete_edge(id: i64) -> i64 {
    delete_edge_inner(id)
}

pub fn delete_edge_inner(eid: i64) -> i64 {
    let tid = id::id_type(eid);
    let endpoints = Spi::connect(|client| {
        client
            .select(
                "SELECT src, dst FROM og_data.og_edge WHERE id = $1",
                None,
                &[eid.into()],
            )
            .expect("edge lookup failed")
            .next()
            .map(|r| (r.get::<i64>(1).unwrap().unwrap(), r.get::<i64>(2).unwrap().unwrap()))
    });
    let Some((src, dst)) = endpoints else { return 0 };

    adjacency::remove(src, tid, 'o', eid);
    adjacency::remove(dst, tid, 'i', eid);

    if let Some(table) = types::storage_table(tid) {
        Spi::run_with_args(&format!("DELETE FROM {table} WHERE id = $1"), &[eid.into()])
            .expect("edge delete failed");
    }
    Spi::run_with_args("DELETE FROM og_data.og_edge WHERE id = $1", &[eid.into()])
        .expect("registry delete failed");
    Spi::run_with_args("DELETE FROM og_data.og_role_player WHERE edge_id = $1", &[eid.into()]).ok();
    crate::stats::relationship_deleted();
    1
}

/// Attach an extra role player for an n-ary relation — spec 002 FR-005.
#[pg_extern]
fn og_add_role_player(graph: &str, rel_type: &str, edge_id: i64, role: &str, player: i64) {
    let gid = types::graph_id(graph);
    let rid = types::type_id(gid, rel_type);
    let role_row = crate::spiu::two::<i32, i32>(
        "SELECT role_id, coalesce(player_type_id, 0) FROM og_catalog.role
          WHERE rel_type_id = ANY($1) AND name = $2",
        &[labeling::og_supertypes(rid).into(), role.into()],
    )
    .expect("role lookup failed");
    let (Some(role_id), player_type) = role_row else {
        error!("relation '{rel_type}' has no role named '{role}'");
    };
    if let Some(pt) = player_type {
        if pt != 0 && !labeling::og_is_subtype(id::id_type(player), pt) {
            error!(
                "role '{role}' requires a '{}', got '{}'",
                type_name_of(pt),
                type_name_of(id::id_type(player))
            );
        }
    }
    Spi::run_with_args(
        "INSERT INTO og_data.og_role_player (edge_id, role_id, player_id) VALUES ($1,$2,$3)
         ON CONFLICT DO NOTHING",
        &[edge_id.into(), role_id.into(), player.into()],
    )
    .expect("role player insert failed");
}
