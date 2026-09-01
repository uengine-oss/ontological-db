//! Per-type union views — the bridge between the type system (002) and the
//! Cypher compiler (003).
//!
//! `MATCH (v:Vehicle)` needs one relation that contains Vehicles, Cars, Trucks
//! and EVs *with their properties as columns*. We materialise that as a view:
//!
//! ```sql
//! CREATE VIEW og_data.v_1 AS
//!   SELECT id, 1::int4 AS type_id, p_model, p_year, NULL::int4 AS p_range_km, __ext FROM og_data.n_1
//!   UNION ALL
//!   SELECT id, 4::int4, p_model, p_year, p_range_km, __ext FROM og_data.n_4
//! ```
//!
//! The descendant set comes from ONE interval-index range scan at compile time
//! (spec 002 FR-009), so the per-row cost of label resolution is zero: by the
//! time the query runs, the label predicate has already become a fixed list of
//! real tables the planner can cost individually.

use crate::catalog::labeling;
use pgrx::prelude::*;
use std::collections::BTreeMap;

pub fn node_view(tid: i32) -> String {
    format!("og_data.v_{tid}")
}

pub fn edge_view(tid: i32) -> String {
    format!("og_data.ve_{tid}")
}

/// (property name, column name, sql type) for every property visible on a type
/// or any of its descendants.
pub fn view_properties(tid: i32) -> BTreeMap<String, (String, String)> {
    let subs = labeling::og_subtypes(tid);
    let mut map = BTreeMap::new();
    Spi::connect(|client| {
        let rows = client
            .select(
                "SELECT name, column_name, data_type FROM og_catalog.property
                  WHERE type_id = ANY($1)",
                None,
                &[subs.into()],
            )
            .expect("property lookup failed");
        for r in rows {
            let name: String = r.get(1).unwrap().unwrap();
            let col: String = r.get(2).unwrap().unwrap();
            let dt: String = r.get(3).unwrap().unwrap();
            map.insert(name, (col, dt));
        }
    });
    map
}

fn concrete_tables(tid: i32, want_edge: bool) -> Vec<(i32, String)> {
    let subs = labeling::og_subtypes(tid);
    Spi::connect(|client| {
        client
            .select(
                // Whether a type is reachable as a node or as an edge is decided
                // by the shape of its storage table, not by its conceptual kind.
                // TypeQL (spec 010) reifies relations and materialises attribute
                // instances into node tables; those are nodes to Cypher, and
                // saying so here is what makes spec 010 FR-040 true.
                "SELECT type_id, storage_table FROM og_catalog.type
                  WHERE type_id = ANY($1) AND storage_table IS NOT NULL
                    AND ((NOT $2 AND (storage_table LIKE 'og_data.n\\_%'
                                      OR storage_table LIKE 'og_data.a\\_%'))
                      OR ($2 AND storage_table LIKE 'og_data.e\\_%'))
                  ORDER BY type_id",
                None,
                &[subs.into(), want_edge.into()],
            )
            .expect("storage table lookup failed")
            .filter_map(|r| Some((r.get::<i32>(1).ok()??, r.get::<String>(2).ok()??)))
            .collect()
    })
}

fn view_exists(name: &str) -> bool {
    let (schema, rel) = name.split_once('.').unwrap_or(("public", name));
    crate::spiu::one::<bool>(
        "SELECT true FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
          WHERE n.nspname = $1 AND c.relname = $2",
        &[schema.into(), rel.into()],
    )
    .unwrap_or(None)
    .unwrap_or(false)
}

/// Create the union view for a type if it does not exist yet. Views are dropped
/// wholesale whenever the schema changes, so "exists" is a valid freshness test.
///
/// The build goes through `og_build_view`, which is `SECURITY DEFINER`. Reading
/// a label is what triggers it, and a reader has no `CREATE` on `og_data` — so
/// before this indirection existed, the first query touching any label after a
/// schema change failed with `permission denied for schema og_data` and stayed
/// broken until someone with rights happened to run it. Building as the owner
/// hands out nothing: the view carries `security_invoker`, so the caller is
/// still checked against the storage tables underneath it.
pub fn ensure_view(tid: i32, is_edge: bool) -> String {
    let view = if is_edge { edge_view(tid) } else { node_view(tid) };
    if view_exists(&view) {
        return view;
    }
    crate::spiu::one::<String>(
        "SELECT og_build_view($1, $2)",
        &[tid.into(), is_edge.into()],
    )
    .ok()
    .flatten()
    .unwrap_or_else(|| build_view(tid, is_edge))
}

/// The view build itself. Reached through `og_build_view` so that it runs with
/// the extension owner's rights; see `ensure_view`.
#[pg_extern]
fn og_build_view(tid: i32, is_edge: bool) -> String {
    build_view(tid, is_edge)
}

fn build_view(tid: i32, is_edge: bool) -> String {
    let view = if is_edge { edge_view(tid) } else { node_view(tid) };
    if view_exists(&view) {
        return view;
    }

    let props = view_properties(tid);
    let tables = concrete_tables(tid, is_edge);

    let mut selects: Vec<String> = Vec::new();
    for (sub, table) in &tables {
        let own = own_columns(table);
        let mut cols: Vec<String> = vec!["id".into(), format!("{sub}::int4 AS type_id")];
        if is_edge {
            cols.push("src".into());
            cols.push("dst".into());
        }
        for (col, dt) in props.values() {
            if own.contains(col) {
                cols.push(col.clone());
            } else {
                cols.push(format!("NULL::{dt} AS {col}"));
            }
        }
        cols.push("__ext".into());
        selects.push(format!("SELECT {} FROM {}", cols.join(", "), table));
    }

    if selects.is_empty() {
        // Abstract type with no concrete descendants: an empty, correctly shaped relation.
        let mut cols: Vec<String> = vec!["NULL::int8 AS id".into(), "NULL::int4 AS type_id".into()];
        if is_edge {
            cols.push("NULL::int8 AS src".into());
            cols.push("NULL::int8 AS dst".into());
        }
        for (col, dt) in props.values() {
            cols.push(format!("NULL::{dt} AS {col}"));
        }
        cols.push("NULL::jsonb AS __ext".into());
        selects.push(format!("SELECT {} WHERE false", cols.join(", ")));
    }

    Spi::run(&format!(
        "CREATE OR REPLACE VIEW {view}{} AS {}",
        crate::spiu::VIEW_SECURITY,
        selects.join("\nUNION ALL\n")
    ))
        .unwrap_or_else(|e| error!("failed to build type view for type {tid}: {e}"));
    crate::catalog::privileges::apply_to_view(tid, &view);
    view
}

fn own_columns(table: &str) -> Vec<String> {
    let (schema, rel) = table.split_once('.').unwrap_or(("public", table));
    Spi::connect(|client| {
        client
            .select(
                "SELECT a.attname::text FROM pg_attribute a
                   JOIN pg_class c ON c.oid = a.attrelid
                   JOIN pg_namespace n ON n.oid = c.relnamespace
                  WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped",
                None,
                &[schema.into(), rel.into()],
            )
            .expect("column lookup failed")
            .filter_map(|r| r.get::<String>(1).ok().flatten())
            .collect()
    })
}

/// Invalidate every generated view. Called on any schema change.
pub fn drop_all_views() {
    let views: Vec<String> = Spi::connect(|client| {
        client
            .select(
                "SELECT 'og_data.' || c.relname FROM pg_class c
                   JOIN pg_namespace n ON n.oid = c.relnamespace
                  WHERE n.nspname = 'og_data' AND c.relkind = 'v'
                    AND (c.relname LIKE 'v\\_%' OR c.relname LIKE 've\\_%')",
                None,
                &[],
            )
            .expect("view scan failed")
            .filter_map(|r| r.get::<String>(1).ok().flatten())
            .collect()
    });
    for v in views {
        Spi::run(&format!("DROP VIEW IF EXISTS {v} CASCADE")).ok();
    }
}
