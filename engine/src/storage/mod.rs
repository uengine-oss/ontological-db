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
pub mod traverse;

use crate::catalog::{labeling, types};
use crate::id;
use crate::layout::WriteError;
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
    /// Does the payload carry anything that is not a declared column? Only
    /// then does a write touch `__ext` — which matters now that writing it is
    /// a privilege of its own (spec 014: a writer granted named properties
    /// holds no privilege on the overflow payload).
    ext_needed: bool,
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
    // Only a key that is new, or whose column must widen, needs anything done —
    // and only then is it worth asking whether this role may do it.
    let needs_work = obj.iter().any(|(key, value)| match infer_column_type(value) {
        None => false,
        Some(want) => match existing.iter().find(|(n, _, _)| n == key) {
            None => true,
            Some((_, _, dtype)) => WIDENABLE.contains(&dtype.as_str()) && !type_accepts(dtype, want),
        },
    });
    if !needs_work || !crate::security::can_change_schema() {
        // A writer may not change the schema. Undeclared values stay in
        // `__ext`, which is what ADR-025 always said happened here.
        return false;
    }

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
                        crate::temporal::sync_history(sub);
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
    crate::security::check_reserved_keys(props);
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

    let mut plan = PropPlan { columns: vec![], exprs: vec![], declared: vec![], ext_needed: false };
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
    plan.ext_needed = props
        .as_object()
        .map(|o| o.keys().any(|k| !plan.declared.contains(k)))
        .unwrap_or(false);
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

/// A virtual type's storage is a view over rows that live elsewhere. The view
/// would pass an UPDATE or DELETE on to the source if the fabric account had
/// the right to, so the refusal has to happen here, not depend on that grant.
pub fn refuse_virtual_write(tid: i32) {
    if let Some(source) = crate::fabric::virtual_source(tid) {
        let name = types::try_type_name(tid).unwrap_or_else(|| tid.to_string());
        error!(
            "'{name}' is a virtual node type and read-only: its rows live in {source}. \
             Write to the source system, or MATERIALIZE VIRTUAL NODE {name} to take a copy"
        );
    }
}

pub fn create_node_inner(_gid: i32, tid: i32, type_name: &str, props: Value) -> i64 {
    if types::type_kind(tid) != 'e' {
        error!("'{type_name}' is not an entity type");
    }
    let table = types::storage_table(tid)
        .unwrap_or_else(|| error!("'{type_name}' is abstract and cannot be instantiated"));
    refuse_virtual_write(tid);

    let plan = plan_props(tid, &props, "$2");
    let ext = ext_expr(&plan, "$2");

    let mut cols = vec!["id".to_string()];
    let mut vals = vec!["$1".to_string()];
    cols.extend(plan.columns.iter().cloned());
    vals.extend(plan.exprs.iter().cloned());
    if plan.ext_needed {
        cols.push("__ext".into());
        vals.push(ext);
    }
    crate::security::check_write(tid, &table, crate::security::WriteOp::Create, &cols);
    let nid = alloc_id(tid);

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
    refuse_virtual_write(tid);
    let table = types::storage_table(tid).unwrap_or_else(|| error!("unknown type for id {id}"));
    let plan = plan_props(tid, &props, "$2");
    let written = props.clone();
    let mut touched = plan.columns.clone();
    if plan.ext_needed {
        touched.push("__ext".into());
    }
    crate::security::check_write(tid, &table, crate::security::WriteOp::Set, &touched);
    if !plan.ext_needed && plan.columns.is_empty() {
        return;
    }
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
    let sql = if plan.ext_needed {
        format!(
            "UPDATE {table} SET {}, __ext = COALESCE(__ext,'{{}}'::jsonb) || COALESCE({ext},'{{}}'::jsonb) WHERE id = $1",
            sets.join(", ")
        )
    } else {
        format!("UPDATE {table} SET {} WHERE id = $1", sets.join(", "))
    };
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
    let mut touched = plan.columns.clone();
    if plan.ext_needed {
        touched.push("__ext".into());
    }
    crate::security::check_write(tid, &table, crate::security::WriteOp::Set, &touched);
    let mut sets: Vec<String> = plan
        .columns
        .iter()
        .zip(plan.exprs.iter())
        .map(|(c, e)| format!("{c} = {e}"))
        .collect();
    if plan.ext_needed {
        sets.push(format!(
            "__ext = COALESCE(__ext,'{{}}'::jsonb) || COALESCE({ext},'{{}}'::jsonb)"
        ));
    }
    if sets.is_empty() {
        return;
    }
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
    refuse_virtual_write(tid);
    if let Some(table) = types::storage_table(tid) {
        crate::security::check_write(tid, &table, crate::security::WriteOp::Delete, &[]);
    }
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

    let plan = plan_props(tid, &props, "$4");
    let ext = ext_expr(&plan, "$4");

    let mut cols = vec!["id".to_string(), "src".into(), "dst".into()];
    let mut vals = vec!["$1".to_string(), "$2".into(), "$3".into()];
    cols.extend(plan.columns.iter().cloned());
    vals.extend(plan.exprs.iter().cloned());
    if plan.ext_needed {
        cols.push("__ext".into());
        vals.push(ext);
    }
    crate::security::check_write(tid, &table, crate::security::WriteOp::Create, &cols);
    let eid = alloc_id(tid);

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
    if let Some(table) = types::storage_table(tid) {
        crate::security::check_write(tid, &table, crate::security::WriteOp::Delete, &[]);
    }

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

/// What a type rename (Cypher's `REMOVE n:Old SET n:New`, `cypher::rename_label`)
/// asks of storage once the catalog row carries the new name: the type's alias
/// view is named after the type, so it moves with it.
pub fn type_renamed(tid: i32, old: &str, new: &str) {
    types::drop_alias_view(old);
    if let Some(table) = types::storage_table(tid) {
        types::ensure_alias_view(tid, new, &table);
    }
}

// --------------------------------------------------------------------------
// TypeQL instances, attributes, ownerships, role players (spec 010)
//
// The statements `typeql/write.rs` ran itself, moved here unchanged — text,
// order, what a failure does — so that TypeQL writes go through the storage
// API (`Layout`, DESIGN 5.1; P1 K3 names it, K4 moves write.rs onto it). The
// catalog checks around them (type kind, `owns`, value type and constraints,
// roles) and the error wording stay in write.rs; a failure comes back as the
// database's text. `table` is the type's `og_catalog.type.storage_table`, and
// `lit` a SQL literal of the attribute's value type that write.rs built.
// --------------------------------------------------------------------------

/// A bare instance — registry row, then the type's own row with only `id`.
/// `typeql::write::create_instance` after its checks.
pub fn create_instance(tid: i32, table: &str) -> Result<i64, WriteError> {
    let id = alloc_id(tid);
    Spi::run_with_args(
        "INSERT INTO og_data.og_node (id, type_id) VALUES ($1, $2)",
        &[id.into(), tid.into()],
    )
    .map_err(|e| WriteError::Register(e.to_string()))?;
    Spi::run_with_args(&format!("INSERT INTO {table} (id) VALUES ($1)"), &[id.into()])
        .map_err(|e| WriteError::Store(e.to_string()))?;
    Ok(id)
}

/// The attribute instance holding `lit`. `typeql::write::intern_attribute`'s lookup.
pub fn find_attribute(table: &str, lit: &str) -> Result<Option<i64>, String> {
    crate::spiu::one::<i64>(
        &format!("SELECT id FROM {table} WHERE val = {lit}"),
        &[],
    )
    .map_err(|e| e.to_string())
}

/// A new attribute instance holding `lit`. `typeql::write::intern_attribute`'s create.
pub fn create_attribute(tid: i32, table: &str, lit: &str) -> Result<i64, WriteError> {
    let id = alloc_id(tid);
    Spi::run_with_args(
        "INSERT INTO og_data.og_node (id, type_id) VALUES ($1, $2)",
        &[id.into(), tid.into()],
    )
    .map_err(|e| WriteError::Register(e.to_string()))?;
    Spi::run_with_args(
        &format!("INSERT INTO {table} (id, val) VALUES ($1, {lit})"),
        &[id.into()],
    )
    .map_err(|e| WriteError::Store(e.to_string()))?;
    Ok(id)
}

/// Another owner (not `except`, of one of `owners`) of an attribute in `table`
/// holding `lit` through `has_tid` — `check_ownership_constraints`'s @key/@unique query.
pub fn other_owner(has_tid: i32, table: &str, lit: &str, except: i64, owners: &[i32]) -> Result<Option<i64>, String> {
    crate::spiu::one::<i64>(
        &format!(
            "SELECT e.src FROM og_data.og_edge e
                   JOIN {table} a ON a.id = e.dst
                  WHERE e.type_id = $1 AND a.val = {lit} AND e.src <> $2
                    AND ((e.src >> {}) & {})::int4 = ANY($3)
                  LIMIT 1",
            crate::id::TYPE_SHIFT,
            crate::id::TYPE_MASK
        ),
        &[has_tid.into(), except.into(), owners.to_vec().into()],
    )
    .map_err(|e| e.to_string())
}

/// How many attributes of one of `attrs` `owner` owns through `has_tid` —
/// `check_ownership_constraints`'s @card query.
pub fn count_owned(has_tid: i32, owner: i64, attrs: &[i32]) -> Result<i64, String> {
    crate::spiu::one::<i64>(
        &format!(
            "SELECT count(*) FROM og_data.og_edge e
                      WHERE e.type_id = $1 AND e.src = $2
                        AND ((e.dst >> {}) & {})::int4 = ANY($3)",
            crate::id::TYPE_SHIFT,
            crate::id::TYPE_MASK
        ),
        &[has_tid.into(), owner.into(), attrs.to_vec().into()],
    )
    .map_err(|e| e.to_string())
    .map(|n| n.unwrap_or(0))
}

/// The attributes of one of `attrs` that `owner` owns through `has_tid` —
/// `typeql::write::run_update`'s lookup.
pub fn owned_attributes(has_tid: i32, owner: i64, attrs: &[i32]) -> Vec<i64> {
    Spi::connect(|client| {
        client
            .select(
                &format!(
                    "SELECT dst FROM og_data.og_edge
                          WHERE type_id = $1 AND src = $2
                            AND ((dst >> {}) & {})::int4 = ANY($3)",
                    crate::id::TYPE_SHIFT,
                    crate::id::TYPE_MASK
                ),
                None,
                &[has_tid.into(), owner.into(), attrs.to_vec().into()],
            )
            .map(|rows| rows.filter_map(|r| r.get::<i64>(1).ok().flatten()).collect())
            .unwrap_or_default()
    })
}

/// The ownerships through `has_tid` that touch `id` either way, `(id, src, dst)` —
/// `typeql::write::delete_instance`'s first lookup.
pub fn ownerships_of(has_tid: i32, id: i64) -> Vec<(i64, i64, i64)> {
    Spi::connect(|client| {
        client
            .select(
                "SELECT id, src, dst FROM og_data.og_edge
                  WHERE type_id = $1 AND (src = $2 OR dst = $2)",
                None,
                &[has_tid.into(), id.into()],
            )
            .map(|rows| {
                rows.filter_map(|r| {
                    Some((
                        r.get::<i64>(1).ok()??,
                        r.get::<i64>(2).ok()??,
                        r.get::<i64>(3).ok()??,
                    ))
                })
                .collect()
            })
            .unwrap_or_default()
    })
}

/// Ownership edge: registry + typed table + both adjacency directions, in one
/// transaction (spec 001 FR-012), unless it exists — `typeql::write::link_has`
/// after `schema::ensure_has_type`.
pub fn link_has(has_tid: i32, owner: i64, attr: i64) -> Result<(), WriteError> {
    let exists = crate::spiu::one::<i64>(
        "SELECT id FROM og_data.og_edge WHERE type_id = $1 AND src = $2 AND dst = $3 LIMIT 1",
        &[has_tid.into(), owner.into(), attr.into()],
    )
    .ok()
    .flatten();
    if exists.is_some() {
        return Ok(());
    }
    let table = types::storage_table(has_tid).expect("$has has no table");
    let eid = alloc_id(has_tid);
    Spi::run_with_args(
        "INSERT INTO og_data.og_edge (id, type_id, src, dst) VALUES ($1, $2, $3, $4)",
        &[eid.into(), has_tid.into(), owner.into(), attr.into()],
    )
    .map_err(|e| WriteError::Register(e.to_string()))?;
    Spi::run_with_args(
        &format!("INSERT INTO {table} (id, src, dst) VALUES ($1, $2, $3)"),
        &[eid.into(), owner.into(), attr.into()],
    )
    .map_err(|e| WriteError::Store(e.to_string()))?;
    adjacency::append(owner, has_tid, 'o', attr, eid);
    adjacency::append(attr, has_tid, 'i', owner, eid);
    Ok(())
}

/// Remove that ownership edge if there is one; failures ignored —
/// `typeql::write::unlink_has` after `schema::ensure_has_type`.
pub fn unlink_has(has_tid: i32, owner: i64, attr: i64) {
    let eid = crate::spiu::one::<i64>(
        "SELECT id FROM og_data.og_edge WHERE type_id = $1 AND src = $2 AND dst = $3",
        &[has_tid.into(), owner.into(), attr.into()],
    )
    .ok()
    .flatten();
    let Some(eid) = eid else { return };
    adjacency::remove(owner, has_tid, 'o', eid);
    adjacency::remove(attr, has_tid, 'i', eid);
    if let Some(table) = types::storage_table(has_tid) {
        Spi::run_with_args(&format!("DELETE FROM {table} WHERE id = $1"), &[eid.into()]).ok();
    }
    Spi::run_with_args("DELETE FROM og_data.og_edge WHERE id = $1", &[eid.into()]).ok();
}

/// Whether relation instance `rel` has a player in `role` —
/// `typeql::write::add_role_player`'s positional search. The value is read as
/// `int8` exactly as there.
pub fn role_filled(rel: i64, role: i32) -> bool {
    crate::spiu::one::<i64>(
        "SELECT 1 FROM og_data.og_role_player WHERE edge_id = $1 AND role_id = $2",
        &[rel.into(), role.into()],
    )
    .ok()
    .flatten()
    .is_some()
}

/// Add a role player, idempotently — `typeql::write::add_role_player`'s insert
/// (its text; `og_add_role_player` above writes the same row with its own).
pub fn add_role_player(rel: i64, role: i32, player: i64) -> Result<(), String> {
    Spi::run_with_args(
        "INSERT INTO og_data.og_role_player (edge_id, role_id, player_id)
         VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        &[rel.into(), role.into(), player.into()],
    )
    .map_err(|e| e.to_string())
}

/// Take `player` out of `rel` in any of `roles` — `typeql::write::run_delete`'s
/// `links` removal.
pub fn remove_role_player(rel: i64, player: i64, roles: &[i32]) -> Result<(), String> {
    Spi::run_with_args(
        "DELETE FROM og_data.og_role_player
                      WHERE edge_id = $1 AND player_id = $2 AND role_id = ANY($3)",
        &[rel.into(), player.into(), roles.to_vec().into()],
    )
    .map_err(|e| e.to_string())
}

/// The relation instances `player` plays a role in —
/// `typeql::write::delete_instance`'s second lookup.
pub fn relations_played(player: i64) -> Vec<i64> {
    Spi::connect(|client| {
        client
            .select(
                "SELECT DISTINCT edge_id FROM og_data.og_role_player WHERE player_id = $1",
                None,
                &[player.into()],
            )
            .map(|rows| rows.filter_map(|r| r.get::<i64>(1).ok().flatten()).collect())
            .unwrap_or_default()
    })
}

/// An instance's own storage, once its ownerships and the relations it plays
/// in are gone — the tail of `typeql::write::delete_instance`, in its order
/// (the virtual-type refusal comes after the role players, as there).
pub fn remove_instance(id: i64) {
    let tid = id::id_type(id);
    Spi::run_with_args("DELETE FROM og_data.og_role_player WHERE edge_id = $1", &[id.into()]).ok();

    refuse_virtual_write(tid);
    if let Some(table) = types::storage_table(tid) {
        Spi::run_with_args(&format!("DELETE FROM {table} WHERE id = $1"), &[id.into()]).ok();
    }
    Spi::run_with_args("DELETE FROM og_data.og_node WHERE id = $1", &[id.into()]).ok();
    Spi::run_with_args("DELETE FROM og_data.og_adj WHERE src = $1", &[id.into()]).ok();
}
