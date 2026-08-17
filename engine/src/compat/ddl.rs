//! `CREATE INDEX` / `CREATE CONSTRAINT` / `DROP …`, executed as Neo4j means them.
//!
//! Neo4j applications issue this DDL at startup, usually with `IF NOT EXISTS`,
//! and then query the index *by the name they gave it*. Two things follow: the
//! statements must succeed, and the name must be remembered. The name lives in
//! `og_catalog.compat_index`; the index itself is an ordinary index on the
//! typed column, because that is what an index is here.

use crate::catalog::{labeling, types};
use crate::cypher::ast::*;
use crate::cypher::eval;
use pgrx::prelude::*;
use serde_json::{json, Value};

/// Run one DDL statement. Returns the rows the clause contributes — none, as in
/// Neo4j, where schema commands report through the summary rather than the
/// stream.
pub fn run(graph: &str, stmt: &Ddl, params: &Value) -> Vec<Value> {
    match stmt {
        Ddl::CreateIndex { name, kind, on_relationship, label, props, options, if_not_exists } => {
            create_index(
                graph,
                name.as_deref(),
                *kind,
                *on_relationship,
                label,
                props,
                options,
                *if_not_exists,
                params,
            )
        }
        Ddl::CreateConstraint { name, label, props, kind, if_not_exists } => {
            create_constraint(graph, name.as_deref(), label, props, *kind, *if_not_exists)
        }
        Ddl::Drop { name, if_exists, constraint } => drop_named(graph, name, *if_exists, *constraint),
    }
    // Counted here rather than in each branch: every one of them either returns
    // having done the thing, or raises and never reaches this line.
    match stmt {
        Ddl::CreateIndex { .. } => crate::stats::index_added(),
        Ddl::CreateConstraint { .. } => crate::stats::constraint_added(),
        Ddl::Drop { constraint: true, .. } => crate::stats::constraint_removed(),
        Ddl::Drop { constraint: false, .. } => crate::stats::index_removed(),
    }
    Vec::new()
}

/// A Neo4j index name resolved back to what it actually indexes.
pub struct IndexEntry {
    pub kind: String,
    pub type_name: String,
    pub props: Vec<String>,
    pub options: Value,
}

pub fn lookup(gid: i32, name: &str) -> Option<IndexEntry> {
    Spi::connect(|client| {
        let mut rows = client
            .select(
                "SELECT kind, type_name, props, options FROM og_catalog.compat_index
                  WHERE graph_id = $1 AND name = $2",
                Some(1),
                &[gid.into(), name.into()],
            )
            .ok()?;
        let r = rows.next()?;
        Some(IndexEntry {
            kind: r.get::<String>(1).ok()??,
            type_name: r.get::<String>(2).ok()??,
            props: r
                .get::<Vec<Option<String>>>(3)
                .ok()??
                .into_iter()
                .flatten()
                .collect(),
            options: r
                .get::<pgrx::JsonB>(4)
                .ok()
                .flatten()
                .map(|j| j.0)
                .unwrap_or(Value::Null),
        })
    })
}

fn exists(gid: i32, name: &str) -> bool {
    crate::spiu::one::<bool>(
        "SELECT true FROM og_catalog.compat_index WHERE graph_id = $1 AND name = $2",
        &[gid.into(), name.into()],
    )
    .ok()
    .flatten()
    .unwrap_or(false)
}

fn register(
    gid: i32,
    name: &str,
    kind: &str,
    on_relationship: bool,
    type_name: &str,
    props: &[String],
    options: &Value,
) {
    let props: Vec<Option<String>> = props.iter().cloned().map(Some).collect();
    Spi::run_with_args(
        "INSERT INTO og_catalog.compat_index (name, graph_id, kind, entity, type_name, props, options)
         VALUES ($1, $2, $3, $4::text::\"char\", $5, $6, $7)
         ON CONFLICT (graph_id, name) DO UPDATE
            SET kind = EXCLUDED.kind, type_name = EXCLUDED.type_name,
                props = EXCLUDED.props, options = EXCLUDED.options",
        &[
            name.into(),
            gid.into(),
            kind.into(),
            (if on_relationship { "r" } else { "e" }).to_string().into(),
            type_name.into(),
            props.into(),
            pgrx::JsonB(options.clone()).into(),
        ],
    )
    .unwrap_or_else(|e| error!("failed to record index '{name}': {e}"));
}

/// Declare a property if it is not declared yet.
///
/// Indexing or constraining a property the type has never been written with is
/// ordinary Cypher — Neo4j has no schema to declare against. Here there is one,
/// so the column is created on the way. Doing it twice is not an error, and
/// `og_add_property` refuses a redeclaration, so the check comes first.
fn ensure_property(graph: &str, tid: i32, label: &str, prop: &str) {
    let declared = crate::spiu::one::<bool>(
        "SELECT true FROM og_catalog.property WHERE type_id = $1 AND name = $2",
        &[tid.into(), prop.into()],
    )
    .ok()
    .flatten()
    .unwrap_or(false);

    if !declared {
        Spi::run_with_args(
            "SELECT og_add_property($1, $2, $3, 'string', false, false)",
            &[graph.into(), label.into(), prop.into()],
        )
        .unwrap_or_else(|e| error!("failed to declare '{label}.{prop}': {e}"));
    }
}

/// Enforce uniqueness over a property *set*, as one index.
///
/// `REQUIRE (t.db, t.schema, t.name) IS NODE KEY` constrains the combination.
/// One unique index per property would be a different, much stronger rule — it
/// would reject two tables of the same name in different schemas, which is the
/// ordinary case this constraint exists to allow.
fn enforce_unique(tid: i32, label: &str, props: &[String], name: &str) {
    let cols: Vec<String> = props.iter().map(|p| types::column_name(p)).collect();
    for sub in labeling::og_subtypes(tid) {
        let Some(table) = types::storage_table(sub) else { continue };
        let idx = format!("uq_{sub}_{}", sanitize(name));
        Spi::run(&format!(
            "CREATE UNIQUE INDEX IF NOT EXISTS {idx} ON {table} ({})",
            cols.join(", ")
        ))
        .unwrap_or_else(|e| error!("failed to enforce uniqueness on '{label}': {e}"));
    }
}

/// A default name in Neo4j's shape, for `CREATE INDEX FOR …` with no name.
fn default_name(kind: &str, label: &str, props: &[String]) -> String {
    format!("{kind}_{label}_{}", props.join("_"))
}

#[allow(clippy::too_many_arguments)]
fn create_index(
    graph: &str,
    name: Option<&str>,
    kind: IndexKind,
    on_relationship: bool,
    label: &str,
    props: &[String],
    options: &[(String, Expr)],
    if_not_exists: bool,
    params: &Value,
) {
    let gid = types::graph_id(graph);
    let kind_name = match kind {
        IndexKind::Btree => "btree",
        IndexKind::Vector => "vector",
        IndexKind::Fulltext => "fulltext",
    };
    let name = name.map(str::to_string).unwrap_or_else(|| default_name(kind_name, label, props));
    if exists(gid, &name) {
        if if_not_exists {
            return;
        }
        error!("an index named '{name}' already exists in graph '{graph}'");
    }

    // The label may not have been written yet — an application that indexes
    // before it inserts is doing the ordinary thing, not a mistake.
    let ty_kind = if on_relationship { "relation" } else { "entity" };
    let tid = types::resolve_or_create_label_set(gid, graph, &[label.to_string()], ty_kind);

    let opts: Value = options
        .iter()
        .map(|(k, v)| (k.clone(), eval::eval(v, &eval::Env::new(), params).unwrap_or(Value::Null)))
        .collect::<serde_json::Map<_, _>>()
        .into();

    match kind {
        IndexKind::Vector => {
            let dims = opts
                .get("vector.dimensions")
                .and_then(Value::as_i64)
                .unwrap_or_else(|| error!(
                    "CREATE VECTOR INDEX needs OPTIONS {{indexConfig: {{`vector.dimensions`: N}}}}"
                )) as i32;
            let metric = opts
                .get("vector.similarity_function")
                .and_then(Value::as_str)
                .unwrap_or("cosine")
                .to_ascii_lowercase();
            let prop = props.first().unwrap_or_else(|| error!("a vector index needs one property"));
            Spi::run_with_args(
                "SELECT og_add_embedding($1, $2, $3, $4, $5)",
                &[graph.into(), label.into(), prop.as_str().into(), dims.into(), metric.into()],
            )
            .unwrap_or_else(|e| error!("failed to create vector index '{name}': {e}"));
        }
        IndexKind::Fulltext => {
            // Indexing a property the type has never been written with is
            // ordinary — declare it so there is a column to index, exactly as
            // the constraint path does.
            for p in props {
                ensure_property(graph, tid, label, p);
            }
            build_fulltext(tid, props, &name);
        }
        IndexKind::Btree => {
            for p in props {
                Spi::run_with_args(
                    "SELECT og_create_index($1, $2, $3)",
                    &[graph.into(), label.into(), p.as_str().into()],
                )
                .unwrap_or_else(|e| error!("failed to create index '{name}': {e}"));
            }
        }
    }
    register(gid, &name, kind_name, on_relationship, label, props, &opts);
}

/// Full-text search over the declared columns.
///
/// This is **not equivalent to Neo4j's full-text index.** It uses PostgreSQL's
/// `simple` dictionary, which does no stemming and no CJK segmentation, so
/// recall differs — most visibly for Korean. Documented rather than hidden:
/// `docs/cypher.md`, "Known differences".
fn build_fulltext(tid: i32, props: &[String], name: &str) {
    for sub in labeling::og_subtypes(tid) {
        let Some(table) = types::storage_table(sub) else { continue };
        let expr = fulltext_expr(props);
        let idx = format!("ftx_{sub}_{}", sanitize(name));
        Spi::run(&format!(
            "CREATE INDEX IF NOT EXISTS {idx} ON {table} USING gin (({expr}))"
        ))
        .unwrap_or_else(|e| error!("failed to build full-text index '{name}': {e}"));
    }
}

/// The `tsvector` a full-text index and its queries must agree on.
pub fn fulltext_expr(props: &[String]) -> String {
    let cols: Vec<String> = props
        .iter()
        .map(|p| format!("coalesce({}::text, '')", types::column_name(p)))
        .collect();
    format!("to_tsvector('simple', {})", cols.join(" || ' ' || "))
}

fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

fn create_constraint(
    graph: &str,
    name: Option<&str>,
    label: &str,
    props: &[String],
    kind: ConstraintKind,
    if_not_exists: bool,
) {
    let gid = types::graph_id(graph);
    let name = name
        .map(str::to_string)
        .unwrap_or_else(|| default_name("constraint", label, props));
    if exists(gid, &name) {
        if if_not_exists {
            return;
        }
        error!("a constraint named '{name}' already exists in graph '{graph}'");
    }
    let tid = types::resolve_or_create_label_set(gid, graph, &[label.to_string()], "entity");

    for p in props {
        ensure_property(graph, tid, label, p);
    }
    if matches!(kind, ConstraintKind::Unique | ConstraintKind::NodeKey) {
        enforce_unique(tid, label, props, &name);
    }
    // Existence (`IS NOT NULL`, and the existence half of `IS NODE KEY`) is
    // recorded but not enforced as a column NOT NULL. Two reasons, both
    // observed rather than assumed:
    //
    //   * PostgreSQL checks NOT NULL per statement; Neo4j checks constraints at
    //     commit. `MERGE (t:Table {name, schema})` followed by `SET t.db = …`
    //     in the same transaction is legal there and would fail here.
    //   * `IS NODE KEY` is a Neo4j *Enterprise* feature. On Community the
    //     statement fails and the application carries on without it, so
    //     enforcing it here makes this database stricter than the one those
    //     applications are actually written against.
    //
    // Uniqueness is enforced, because that half is checkable at write time and
    // is what callers rely on for MERGE to be idempotent.
    register(gid, &name, "constraint", false, label, props, &json!({}));
}

fn drop_named(graph: &str, name: &str, if_exists: bool, constraint: bool) {
    let gid = types::graph_id(graph);
    if !exists(gid, name) {
        if if_exists {
            return;
        }
        let what = if constraint { "constraint" } else { "index" };
        error!("no {what} named '{name}' in graph '{graph}'");
    }
    // The catalog entry goes; the underlying column and its index stay. Dropping
    // a declared property would drop its data, which `DROP INDEX` never means.
    Spi::run_with_args(
        "DELETE FROM og_catalog.compat_index WHERE graph_id = $1 AND name = $2",
        &[gid.into(), name.into()],
    )
    .unwrap_or_else(|e| error!("failed to drop '{name}': {e}"));
}
