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
use crate::cypher::compile::json_object;
use pgrx::prelude::*;
use std::cell::Cell;
use std::collections::BTreeMap;

thread_local! {
    /// Set when this backend created a view since the flag was last taken.
    /// A view created inside a transaction disappears if that transaction
    /// rolls back, so SQL compiled against it must not be cached — see
    /// `compile_cached`.
    static CREATED_VIEW: Cell<bool> = const { Cell::new(false) };
}

/// Whether a view was created since the last call, clearing the flag.
pub fn take_created_view() -> bool {
    CREATED_VIEW.with(|c| c.replace(false))
}

/// The view of a type's nodes: `v_<id>`, or `vp_<id>` when its properties are
/// packed (a wide view, see [`Layout`]). The names differ so that a view built
/// before packing existed — every property a column — is never read as a
/// packed one: a database upgraded in place still holds such views until the
/// next schema change drops them.
pub fn node_view(tid: i32) -> String {
    if is_packed(tid) {
        format!("og_data.vp_{tid}")
    } else {
        format!("og_data.v_{tid}")
    }
}

pub fn edge_view(tid: i32) -> String {
    if is_packed(tid) {
        format!("og_data.vep_{tid}")
    } else {
        format!("og_data.ve_{tid}")
    }
}

/// Whether `tid`'s view packs properties into `__ext`.
fn is_packed(tid: i32) -> bool {
    !view_layout(tid).packed.is_empty()
}

/// (property name, column name, sql type) for every property visible on a type
/// or any of its descendants.
pub fn view_properties(tid: i32) -> BTreeMap<String, (String, String)> {
    property_census(tid).into_iter().map(|(name, (col, dt, _))| (name, (col, dt))).collect()
}

/// [`view_properties`], with how many of the types under `tid` declare each one.
fn property_census(tid: i32) -> BTreeMap<String, (String, String, usize)> {
    let subs = labeling::og_subtypes(tid);
    let mut map: BTreeMap<String, (String, String, usize)> = BTreeMap::new();
    Spi::connect(|client| {
        let rows = client
            .select(
                "SELECT name, column_name, data_type FROM og_catalog.property
                  WHERE type_id = ANY($1) ORDER BY type_id, name",
                None,
                &[subs.into()],
            )
            .expect("property lookup failed");
        for r in rows {
            let name: String = r.get(1).unwrap().unwrap();
            let col: String = r.get(2).unwrap().unwrap();
            let dt: String = r.get(3).unwrap().unwrap();
            // Subtypes may declare the same property with different types
            // (`tier` text on one, int8 on another): the view needs one type
            // per column, or its UNION ALL fails to build.
            match map.get_mut(&name) {
                None => {
                    map.insert(name, (col, dt, 1));
                }
                Some((_, have, n)) => {
                    *n += 1;
                    if *have != dt {
                        *have = common_type(have, &dt);
                    }
                }
            }
        }
    });
    map
}

/// Above this many property columns a type's view stops giving every property
/// a column of its own.
///
/// A label over many types — an application's umbrella supertype over every
/// class it publishes — unions all their properties. Each one a column, that
/// view hit PostgreSQL's 1664-column limit at a few hundred types (and the
/// node's JSON, built from those columns, its 100-argument limit at about 50),
/// and every query over it planned hundreds of columns per member table.
/// Past this width the properties only one type declares travel in `__ext`
/// instead (see [`Layout`]); those several types share keep their columns.
pub const WIDE_VIEW_COLUMNS: usize = 200;

/// Where a type's view keeps each property.
pub struct Layout {
    /// Properties that are columns of the view: name → (column, SQL type).
    pub columns: BTreeMap<String, (String, String)>,
    /// Properties folded into the view's `__ext` (by name, typed as JSON):
    /// name → (column of the member table, SQL type to read it back as).
    pub packed: BTreeMap<String, (String, String)>,
}

impl Layout {
    /// SQL for reading property `prop` of the row `alias`, with its SQL type.
    pub fn read(&self, alias: &str, prop: &str) -> Option<(String, String)> {
        if let Some((col, dt)) = self.columns.get(prop) {
            return Some((format!("{alias}.{col}"), dt.clone()));
        }
        let (_, dt) = self.packed.get(prop)?;
        Some((packed_read(alias, prop, dt), dt.clone()))
    }

    /// SQL for reading property `prop` of `alias` as jsonb, keeping its JSON type.
    pub fn read_json(&self, alias: &str, prop: &str) -> Option<String> {
        if let Some((col, _)) = self.columns.get(prop) {
            return Some(format!("to_jsonb({alias}.{col})"));
        }
        self.packed.get(prop)?;
        Some(format!("({alias}.__ext -> {})", quote(prop)))
    }
}

/// A packed property read back as its declared SQL type.
fn packed_read(alias: &str, prop: &str, dt: &str) -> String {
    let field = format!("{alias}.__ext -> {}", quote(prop));
    if dt == "jsonb" {
        format!("({field})")
    } else if let Some(elem) = dt.strip_suffix("[]") {
        format!(
            "(CASE WHEN jsonb_typeof({field}) = 'array' THEN \
             ARRAY(SELECT x::{elem} FROM jsonb_array_elements_text({field}) x) END)"
        )
    } else {
        format!("(({alias}.__ext ->> {})::{dt})", quote(prop))
    }
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The layout of `tid`'s view: every property a column, unless that would make
/// the view wider than [`WIDE_VIEW_COLUMNS`] — then only properties declared
/// by two or more of its types are columns (the most widely shared first, up
/// to that width) and the rest are packed.
pub fn view_layout(tid: i32) -> Layout {
    let census = property_census(tid);
    // One decision per physical column: a property being renamed has two names
    // for one column (see view_columns).
    let mut per_col: BTreeMap<String, usize> = BTreeMap::new();
    for (col, _, n) in census.values() {
        let e = per_col.entry(col.clone()).or_insert(0);
        *e = (*e).max(*n);
    }
    let keep: std::collections::BTreeSet<String> = if per_col.len() <= WIDE_VIEW_COLUMNS {
        per_col.keys().cloned().collect()
    } else {
        let mut ranked: Vec<(&String, &usize)> = per_col.iter().filter(|(_, n)| **n >= 2).collect();
        ranked.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        ranked.into_iter().take(WIDE_VIEW_COLUMNS).map(|(c, _)| c.clone()).collect()
    };
    let mut layout = Layout { columns: BTreeMap::new(), packed: BTreeMap::new() };
    for (name, (col, dt, _)) in census {
        if keep.contains(&col) {
            layout.columns.insert(name, (col, dt));
        } else {
            layout.packed.insert(name, (col, dt));
        }
    }
    layout
}

/// One SQL type both columns convert to: the wider integer, numeric for mixed
/// numbers, text otherwise.
fn common_type(a: &str, b: &str) -> String {
    const INT: &[&str] = &["int2", "int4", "int8"];
    const NUM: &[&str] = &["int2", "int4", "int8", "numeric", "float4", "float8"];
    if INT.contains(&a) && INT.contains(&b) {
        "int8".into()
    } else if NUM.contains(&a) && NUM.contains(&b) {
        "numeric".into()
    } else {
        "text".into()
    }
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
/// Views are created lazily by whoever queries first, and most roles cannot
/// create anything in `og_data` — so a role without that privilege builds the
/// view through `og__ensure_view`, which runs as the extension owner. The view
/// is `security_invoker`, so who created it does not change who may read
/// through it.
pub fn ensure_view(tid: i32, is_edge: bool) -> String {
    let view = if is_edge { edge_view(tid) } else { node_view(tid) };
    if view_exists(&view) {
        return view;
    }
    let can_create = crate::spiu::one::<bool>(
        "SELECT has_schema_privilege('og_data', 'CREATE')",
        &[],
    )
    .ok()
    .flatten()
    .unwrap_or(false);
    CREATED_VIEW.with(|c| c.set(true));
    if can_create {
        build_view(tid, is_edge)
    } else {
        crate::spiu::one_mut::<String>(
            "SELECT og__ensure_view($1, $2)",
            &[tid.into(), is_edge.into()],
        )
        .unwrap_or_else(|e| error!("failed to build type view for type {tid}: {e}"))
        .unwrap_or(view)
    }
}

/// Build a generated type view as the extension owner. It takes a type id and
/// a flag, and can only ever produce the view `ensure_view` would have built.
#[allow(non_snake_case)]
#[pg_extern(security_definer)]
#[search_path(@extschema@, pg_catalog, pg_temp)]
fn og__ensure_view(tid: i32, is_edge: bool) -> String {
    let view = if is_edge { edge_view(tid) } else { node_view(tid) };
    if view_exists(&view) {
        return view;
    }
    build_view(tid, is_edge)
}

fn build_view(tid: i32, is_edge: bool) -> String {
    let view = if is_edge { edge_view(tid) } else { node_view(tid) };
    // Two sessions that both find the view missing would both create it, and
    // the second fails on a duplicate catalog row. One builds it; the other
    // waits for that to commit and then finds it.
    crate::catalog::labeling::lock(
        if is_edge { crate::catalog::labeling::LOCK_EDGE_VIEW } else { crate::catalog::labeling::LOCK_NODE_VIEW },
        tid,
    );
    if view_exists(&view) {
        return view;
    }
    let layout = view_layout(tid);
    let props = &layout.columns;
    let tables = concrete_tables(tid, is_edge);

    let mut selects: Vec<String> = Vec::new();
    for (sub, table) in &tables {
        // A temporal type contributes its current rows and, when an instant
        // is set, its history rows — both filtered to that instant (spec 013).
        let temporal = crate::temporal::is_temporal(*sub);
        let mut sources: Vec<(String, String)> = Vec::new();
        if temporal {
            let (cur, open, closed) = crate::temporal::visibility_predicates();
            sources.push((table.clone(), format!(" WHERE {cur}")));
            if let Some(h) = crate::temporal::history_table(table) {
                sources.push((h.clone(), format!(" WHERE {open}")));
                sources.push((h, format!(" WHERE {closed}")));
            }
        } else {
            sources.push((table.clone(), String::new()));
        }
        for (rel, filter) in sources {
            let own = own_columns(&rel);
            let mut cols: Vec<String> = vec!["id".into(), format!("{sub}::int4 AS type_id")];
            if is_edge {
                cols.push("src".into());
                cols.push("dst".into());
            }
            for (col, dt) in view_columns(props) {
                if own.contains(&col) {
                    // A no-op when the column already has the view's type;
                    // converts it when subtypes disagree (see view_properties).
                    cols.push(format!("{col}::{dt} AS {col}"));
                } else {
                    cols.push(format!("NULL::{dt} AS {col}"));
                }
            }
            // Packed properties (a wide view, see Layout) ride in `__ext`
            // under their names, as a node's JSON shows them anyway.
            let packed: Vec<String> = layout
                .packed
                .iter()
                .filter(|(_, (col, _))| own.contains(col))
                .map(|(name, (col, _))| format!("{}, to_jsonb({col})", quote(name)))
                .collect();
            if packed.is_empty() {
                cols.push("__ext".into());
            } else {
                cols.push(format!(
                    "(jsonb_strip_nulls({}) || COALESCE(__ext, '{{}}'::jsonb)) AS __ext",
                    json_object(&packed)
                ));
            }
            selects.push(format!("SELECT {} FROM {}{}", cols.join(", "), rel, filter));
        }
    }

    if selects.is_empty() {
        // Abstract type with no concrete descendants: an empty, correctly shaped relation.
        let mut cols: Vec<String> = vec!["NULL::int8 AS id".into(), "NULL::int4 AS type_id".into()];
        if is_edge {
            cols.push("NULL::int8 AS src".into());
            cols.push("NULL::int8 AS dst".into());
        }
        for (col, dt) in view_columns(props) {
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
    crate::catalog::privileges::apply_to_view(&view);
    view
}

/// Each physical column once. While a property is being renamed (spec 012) it
/// has two names for one column, and a view cannot select a column twice.
fn view_columns(props: &BTreeMap<String, (String, String)>) -> Vec<(String, String)> {
    let mut seen = std::collections::BTreeSet::new();
    props
        .values()
        .filter(|(col, _)| seen.insert(col.clone()))
        .cloned()
        .collect()
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
                    AND (c.relname LIKE 'v\\_%' OR c.relname LIKE 've\\_%'
                         OR c.relname LIKE 'vp\\_%' OR c.relname LIKE 'vep\\_%')
                  ORDER BY c.oid",
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
