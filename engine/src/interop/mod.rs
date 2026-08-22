//! PostgreSQL / Supabase interoperability — spec 005.
//!
//! The load-bearing observation: a compiled Cypher query reads ordinary tables.
//! So row-level security applied to those tables applies **mid-traversal**, with
//! no enforcement code of our own — a node the caller cannot see simply fails to
//! join, and every path through it disappears (FR-013). A forked database would
//! have had to build that; an extension inherits it.

use crate::catalog::{labeling, types};
use pgrx::prelude::*;
use pgrx::JsonB;
use serde_json::{json, Value};

/// Enable row-level security on a type and all of its subtypes.
///
/// `policy_expr` is a SQL boolean expression over the type table's columns —
/// e.g. `p_tenant_id = current_setting('app.tenant')::int`.
#[pg_extern]
fn og_enable_rls(graph: &str, type_name: &str, policy_expr: &str) {
    let gid = types::graph_id(graph);
    let tid = types::type_id(gid, type_name);
    for sub in labeling::og_subtypes(tid) {
        let Some(table) = types::storage_table(sub) else { continue };
        // ENABLE on its own exempts the table's owner, and the storage tables
        // are owned by whoever installed the extension — so a policy without
        // FORCE is not evaluated for the one role most likely to be running the
        // query. Both statements, or the isolation claim is not true.
        Spi::run(&format!("ALTER TABLE {table} ENABLE ROW LEVEL SECURITY"))
            .unwrap_or_else(|e| error!("failed to enable RLS on {table}: {e}"));
        Spi::run(&format!("ALTER TABLE {table} FORCE ROW LEVEL SECURITY"))
            .unwrap_or_else(|e| error!("failed to force RLS on {table}: {e}"));
        Spi::run(&format!("DROP POLICY IF EXISTS og_policy ON {table}")).ok();
        Spi::run(&format!(
            "CREATE POLICY og_policy ON {table} USING ({policy_expr})"
        ))
        .unwrap_or_else(|e| error!("failed to create policy on {table}: {e}"));
    }
}

/// PostgREST / supabase-js entry point: one call, one JSON array.
#[pg_extern]
fn og_cypher_json(graph: &str, query: &str, params: default!(JsonB, "'{}'")) -> JsonB {
    let rows: Vec<Value> = Spi::connect(|client| {
        client
            .select(
                "SELECT coalesce(jsonb_agg(r), '[]'::jsonb) FROM og_cypher($1,$2,$3) r",
                None,
                &[graph.into(), query.into(), params.into()],
            )
            .unwrap_or_else(|e| error!("{e}"))
            .next()
            .and_then(|r| r.get::<JsonB>(1).ok().flatten())
            .map(|j| match j.0 {
                Value::Array(a) => a,
                other => vec![other],
            })
            .unwrap_or_default()
    });
    JsonB(Value::Array(rows))
}

/// Map an existing relational table onto a node type without copying data.
///
/// Creates a view shaped like a generated type table, so the Cypher compiler can
/// treat it exactly like native storage (FR-006..FR-009).
#[pg_extern]
fn og_map_table(
    graph: &str,
    source_table: &str,
    type_name: &str,
    id_column: &str,
    property_map: JsonB,
) {
    let gid = types::graph_id(graph);
    let tid = types::type_id(gid, type_name);
    let Value::Object(map) = property_map.0 else {
        error!("property_map must be a JSON object of {{ \"graphProperty\": \"sourceColumn\" }}");
    };

    let mut cols: Vec<String> = vec![format!(
        "(og_make_id(0, {tid}, ({id_column})::int8)) AS id"
    )];
    for (prop, col) in &map {
        let Some(src_col) = col.as_str() else {
            error!("property_map values must be column names");
        };
        // Declare the property if the modeller has not already.
        let dtype = crate::spiu::one::<String>(
            "SELECT data_type FROM og_catalog.property WHERE type_id = $1 AND name = $2",
            &[tid.into(), prop.as_str().into()],
        )
        .ok()
        .flatten()
        .unwrap_or_else(|| {
            error!("declare property '{prop}' on '{type_name}' before mapping a column to it")
        });
        cols.push(format!("({src_col})::{dtype} AS {}", types::column_name(prop)));
    }
    cols.push("NULL::jsonb AS __ext".into());

    let table = types::storage_table(tid)
        .unwrap_or_else(|| error!("'{type_name}' is abstract and cannot back a mapping"));
    Spi::run(&format!("DROP TABLE IF EXISTS {table} CASCADE")).ok();
    Spi::run(&format!(
        "CREATE VIEW {table}{} AS SELECT {} FROM {}",
        crate::spiu::VIEW_SECURITY,
        cols.join(", "),
        crate::spiu::qname(source_table)
    ))
    .unwrap_or_else(|e| error!("failed to create mapping view: {e}"));
    crate::catalog::privileges::apply_to_view(&table);

    Spi::run_with_args(
        "INSERT INTO og_catalog.mapping (type_id, source_table, id_column, property_map, writable)
         VALUES ($1, $2, $3, $4, false)
         ON CONFLICT (type_id) DO UPDATE
            SET source_table = EXCLUDED.source_table, id_column = EXCLUDED.id_column,
                property_map = EXCLUDED.property_map",
        &[tid.into(), source_table.into(), id_column.into(), JsonB(Value::Object(map)).into()],
    )
    .expect("mapping registration failed");

    crate::cypher::views::drop_all_views();
}

/// Materialise a mapped type into native storage — spec 005 FR-010.
///
/// Same Cypher queries, same results, native performance.
#[pg_extern]
fn og_materialize_mapping(graph: &str, type_name: &str) -> i64 {
    let gid = types::graph_id(graph);
    let tid = types::type_id(gid, type_name);
    let table = types::storage_table(tid).unwrap_or_else(|| error!("no storage for '{type_name}'"));

    let is_view = crate::spiu::one::<bool>(
        "SELECT c.relkind = 'v' FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
          WHERE n.nspname = split_part($1,'.',1) AND c.relname = split_part($1,'.',2)",
        &[table.clone().into()],
    )
    .ok()
    .flatten()
    .unwrap_or(false);
    if !is_view {
        error!("'{type_name}' is already stored natively");
    }

    let tmp = format!("{table}_mat");
    Spi::run(&format!("CREATE TABLE {tmp} AS SELECT * FROM {table}"))
        .unwrap_or_else(|e| error!("materialisation failed: {e}"));
    Spi::run(&format!("DROP VIEW {table} CASCADE")).ok();
    Spi::run(&format!("ALTER TABLE {tmp} RENAME TO {}", table.split('.').nth(1).unwrap()))
        .unwrap_or_else(|e| error!("rename failed: {e}"));
    Spi::run(&format!("ALTER TABLE {table} ADD PRIMARY KEY (id)")).ok();

    let n = crate::spiu::one::<i64>(&format!("SELECT count(*) FROM {table}"), &[])
        .ok()
        .flatten()
        .unwrap_or(0);
    Spi::run_with_args(
        "INSERT INTO og_data.og_node (id, type_id) SELECT id, $1 FROM og_data.og_node x
          WHERE false",
        &[tid.into()],
    )
    .ok();
    Spi::run(&format!(
        "INSERT INTO og_data.og_node (id, type_id) SELECT id, {tid} FROM {table}
         ON CONFLICT DO NOTHING"
    ))
    .ok();
    Spi::run_with_args(
        "UPDATE og_catalog.mapping SET writable = true WHERE type_id = $1",
        &[tid.into()],
    )
    .ok();
    crate::cypher::views::drop_all_views();
    n
}

/// Everything a caller needs to know about how the graph is exposed to SQL.
#[pg_extern(stable)]
fn og_interop_report(graph: &str) -> JsonB {
    let gid = types::graph_id(graph);
    let mappings: Vec<Value> = Spi::connect(|client| {
        client
            .select(
                "SELECT t.name, m.source_table, m.id_column, m.writable
                   FROM og_catalog.mapping m JOIN og_catalog.type t ON t.type_id = m.type_id
                  WHERE t.graph_id = $1",
                None,
                &[gid.into()],
            )
            .expect("mapping scan failed")
            .map(|r| {
                json!({
                    "type": r.get::<String>(1).unwrap(),
                    "source_table": r.get::<String>(2).unwrap(),
                    "id_column": r.get::<String>(3).unwrap(),
                    "writable": r.get::<bool>(4).unwrap().unwrap_or(false),
                })
            })
            .collect()
    });

    let secured: Vec<Value> = Spi::connect(|client| {
        client
            .select(
                "SELECT t.name, c.relrowsecurity
                   FROM og_catalog.type t
                   JOIN pg_class c ON c.relname = split_part(t.storage_table, '.', 2)
                   JOIN pg_namespace n ON n.oid = c.relnamespace AND n.nspname = 'og_data'
                  WHERE t.graph_id = $1 AND c.relrowsecurity",
                None,
                &[gid.into()],
            )
            .expect("rls scan failed")
            .map(|r| json!({ "type": r.get::<String>(1).unwrap(), "rls": true }))
            .collect()
    });

    JsonB(json!({
        "graph": graph,
        "relational_views": ["og_node_view", "og_edge_view", "og_type_view", "og_property_view"],
        "rpc_entry_point": "og_cypher_json(graph, query, params)",
        "sql_bridge": "og_cypher_sql(graph, query) returns embeddable SQL",
        "mapped_types": mappings,
        "row_level_security": secured,
    }))
}
