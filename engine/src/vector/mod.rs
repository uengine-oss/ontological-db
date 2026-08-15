//! Vector & hybrid semantic search — spec 004.
//!
//! There is no separate embedding store. An embedding is a `vector(N)` property,
//! which spec 002 turns into a real column on the type table and spec 001 stores
//! like any other column. That single decision is what makes **relationship
//! embeddings first class** (FR-002): an edge type table gets a vector column the
//! same way a node type table does, with the same index, transactions and RLS.
//!
//! It is also what makes filter push-down structural rather than aspirational
//! (FR-013): by the time a vector search runs, the Cypher compiler has already
//! resolved the label to concrete tables, so the graph predicate and the ANN
//! index live on the *same relation*. There is nowhere for a post-filter to hide.

use crate::catalog::{labeling, types};
use pgrx::prelude::*;
use pgrx::JsonB;
use serde_json::{json, Value};

fn metric_op(metric: &str) -> (&'static str, &'static str) {
    match metric.to_ascii_lowercase().as_str() {
        "cosine" => ("<=>", "vector_cosine_ops"),
        "l2" | "euclidean" => ("<->", "vector_l2_ops"),
        "ip" | "inner_product" | "dot" => ("<#>", "vector_ip_ops"),
        other => error!("unknown metric '{other}' (cosine | l2 | ip)"),
    }
}

/// Declare an embedding slot on a node **or relationship** type.
///
/// `source_prop` records what the embedding was derived from so staleness can be
/// detected later (FR-022).
#[pg_extern]
fn og_add_embedding(
    graph: &str,
    type_name: &str,
    prop: &str,
    dims: i32,
    metric: default!(&str, "'cosine'"),
    source_prop: default!(Option<&str>, "NULL"),
) {
    if !(1..=16000).contains(&dims) {
        error!("embedding dimension {dims} is out of range (1..16000)");
    }
    let gid = types::graph_id(graph);
    let tid = types::type_id(gid, type_name);
    let (_, opclass) = metric_op(metric);

    // Reuse the ordinary property path: this is the whole point.
    Spi::run_with_args(
        "SELECT og_add_property($1, $2, $3, $4, false, false)",
        &[graph.into(), type_name.into(), prop.into(), format!("vector({dims})").into()],
    )
    .unwrap_or_else(|e| error!("failed to declare embedding property: {e}"));

    let col = types::column_name(prop);
    for sub in labeling::og_subtypes(tid) {
        if let Some(table) = types::storage_table(sub) {
            Spi::run(&format!(
                "CREATE INDEX IF NOT EXISTS hnsw_{sub}_{col} ON {table} \
                 USING hnsw ({col} {opclass})"
            ))
            .unwrap_or_else(|e| error!("failed to build HNSW index: {e}"));
        }
    }

    Spi::run_with_args(
        "INSERT INTO og_catalog.embedding (emb_id, type_id, prop, dims, metric, source_prop)
         VALUES (nextval('og_catalog.embedding_id_seq')::int4, $1, $2, $3, $4, $5)
         ON CONFLICT (type_id, prop) DO UPDATE
            SET dims = EXCLUDED.dims, metric = EXCLUDED.metric,
                source_prop = EXCLUDED.source_prop",
        &[tid.into(), prop.into(), dims.into(), metric.into(), source_prop.into()],
    )
    .expect("embedding registration failed");
}

fn embedding_meta(tid: i32, prop: &str) -> (i32, String) {
    let row = crate::spiu::two::<i32, String>(
        "SELECT e.dims, e.metric FROM og_catalog.embedding e
          WHERE e.type_id = ANY($1) AND e.prop = $2 LIMIT 1",
        &[labeling::og_supertypes(tid).into(), prop.into()],
    )
    .expect("embedding lookup failed");
    match row {
        (Some(d), Some(m)) => (d, m),
        _ => error!("no embedding named '{prop}' is declared on this type"),
    }
}

/// Top-k semantic search over a node **or relationship** type.
///
/// `filter` is a SQL boolean fragment evaluated on the same relation as the
/// index, which is what keeps it a push-down rather than a post-filter.
#[pg_extern]
fn og_vector_search(
    graph: &str,
    type_name: &str,
    prop: &str,
    query: &str,
    k: default!(i32, 10),
    filter: default!(Option<&str>, "NULL"),
) -> TableIterator<
    'static,
    (name!(id, i64), name!(score, f64), name!(entity, JsonB)),
> {
    let gid = types::graph_id(graph);
    let tid = types::type_id(gid, type_name);
    let (dims, metric) = embedding_meta(tid, prop);
    let (op, _) = metric_op(&metric);
    let col = types::column_name(prop);
    let is_edge = types::type_kind(tid) == 'r';
    let view = crate::cypher::views::ensure_view(tid, is_edge);
    let json_fn = if is_edge { "og_edge_json" } else { "og_node_json" };

    let where_sql = match filter {
        Some(f) if !f.trim().is_empty() => format!("AND ({f})"),
        _ => String::new(),
    };
    // Cosine/IP distances are converted to a similarity so higher is always better.
    let score = match op {
        "<=>" => format!("(1 - (v.{col} {op} $1::vector))"),
        "<#>" => format!("(-(v.{col} {op} $1::vector))"),
        _ => format!("(v.{col} {op} $1::vector)"),
    };

    let sql = format!(
        "SELECT v.id, {score}::float8 AS score, {json_fn}(v.id) AS entity
           FROM {view} v
          WHERE v.{col} IS NOT NULL {where_sql}
          ORDER BY v.{col} {op} $1::vector
          LIMIT {k}"
    );

    if query.matches(',').count() + 1 != dims as usize {
        error!(
            "query vector has {} dimension(s) but '{type_name}.{prop}' is declared as vector({dims})",
            query.matches(',').count() + 1
        );
    }

    let rows: Vec<(i64, f64, JsonB)> = Spi::connect(|client| {
        client
            .select(&sql, None, &[query.into()])
            .unwrap_or_else(|e| error!("vector search failed: {e}"))
            .filter_map(|r| {
                Some((
                    r.get::<i64>(1).ok()??,
                    r.get::<f64>(2).ok()??,
                    r.get::<JsonB>(3).ok()?.unwrap_or(JsonB(json!({}))),
                ))
            })
            .collect()
    });
    TableIterator::new(rows)
}

/// "Find things like this one" — works for relationships too (FR-012).
#[pg_extern]
fn og_similar(
    graph: &str,
    id: i64,
    prop: &str,
    k: default!(i32, 10),
) -> TableIterator<'static, (name!(id, i64), name!(score, f64), name!(entity, JsonB))> {
    let tid = crate::id::id_type(id);
    let (_, metric) = embedding_meta(tid, prop);
    let (op, _) = metric_op(&metric);
    let col = types::column_name(prop);
    let is_edge = types::type_kind(tid) == 'r';
    let root = root_type_of(tid);
    let view = crate::cypher::views::ensure_view(root, is_edge);
    let json_fn = if is_edge { "og_edge_json" } else { "og_node_json" };
    let _ = graph;

    let score = match op {
        "<=>" => format!("(1 - (v.{col} {op} q.{col}))"),
        "<#>" => format!("(-(v.{col} {op} q.{col}))"),
        _ => format!("(v.{col} {op} q.{col})"),
    };
    let sql = format!(
        "WITH q AS (SELECT {col} FROM {view} WHERE id = $1)
         SELECT v.id, {score}::float8 AS score, {json_fn}(v.id) AS entity
           FROM {view} v, q
          WHERE v.{col} IS NOT NULL AND v.id <> $1
          ORDER BY v.{col} {op} q.{col}
          LIMIT {k}"
    );
    let rows: Vec<(i64, f64, JsonB)> = Spi::connect(|client| {
        client
            .select(&sql, None, &[id.into()])
            .unwrap_or_else(|e| error!("similarity search failed: {e}"))
            .filter_map(|r| {
                Some((
                    r.get::<i64>(1).ok()??,
                    r.get::<f64>(2).ok()??,
                    r.get::<JsonB>(3).ok()?.unwrap_or(JsonB(json!({}))),
                ))
            })
            .collect()
    });
    TableIterator::new(rows)
}

fn root_type_of(tid: i32) -> i32 {
    crate::spiu::one::<i32>(
        "SELECT a.type_id FROM og_catalog.type_label d
           JOIN og_catalog.type_label a ON a.graph_id = d.graph_id
            AND d.lft >= a.lft AND d.rgt <= a.rgt
          WHERE d.type_id = $1 ORDER BY a.depth ASC LIMIT 1",
        &[tid.into()],
    )
    .ok()
    .flatten()
    .unwrap_or(tid)
}

/// Hybrid ranking — vector similarity fused with graph proximity (FR-018..021).
///
/// `anchor` is a node the results should also be graph-close to. Fusion is
/// Reciprocal Rank Fusion by default: rank-based, so the two scores do not have
/// to share a scale.
#[pg_extern]
fn og_hybrid_search(
    graph: &str,
    type_name: &str,
    prop: &str,
    query: &str,
    anchor: default!(Option<i64>, "NULL"),
    k: default!(i32, 10),
    vector_weight: default!(f64, 1.0),
    graph_weight: default!(f64, 1.0),
) -> TableIterator<
    'static,
    (
        name!(id, i64),
        name!(score, f64),
        name!(vector_score, f64),
        name!(graph_score, f64),
        name!(entity, JsonB),
    ),
> {
    let gid = types::graph_id(graph);
    let tid = types::type_id(gid, type_name);
    let (_, metric) = embedding_meta(tid, prop);
    let (op, _) = metric_op(&metric);
    let col = types::column_name(prop);
    let view = crate::cypher::views::ensure_view(tid, false);
    let pool = (k * 10).max(50);

    // Graph proximity: shortest hop count from the anchor, capped.
    let graph_cte = match anchor {
        Some(a) => format!(
            "prox AS (SELECT node, min(depth) AS hops FROM og_vlp({a}::int8, NULL, 'b'::\"char\", 0, 3) GROUP BY node)"
        ),
        None => "prox AS (SELECT NULL::int8 AS node, NULL::int AS hops WHERE false)".to_string(),
    };
    let score = match op {
        "<=>" => format!("(1 - (v.{col} {op} $1::vector))"),
        "<#>" => format!("(-(v.{col} {op} $1::vector))"),
        _ => format!("(1.0 / (1.0 + (v.{col} {op} $1::vector)))"),
    };

    let sql = format!(
        "WITH {graph_cte},
         cand AS (
            SELECT v.id, {score}::float8 AS vscore,
                   row_number() OVER (ORDER BY v.{col} {op} $1::vector) AS vrank
              FROM {view} v WHERE v.{col} IS NOT NULL
             ORDER BY v.{col} {op} $1::vector LIMIT {pool}),
         fused AS (
            SELECT c.id, c.vscore,
                   COALESCE(1.0 / (1.0 + p.hops), 0)::float8 AS gscore,
                   {vector_weight} * (1.0 / (60 + c.vrank))
                 + {graph_weight}  * COALESCE(1.0 / (60 + p.hops), 0) AS fscore
              FROM cand c LEFT JOIN prox p ON p.node = c.id)
         SELECT id, fscore::float8, vscore::float8, gscore::float8, og_node_json(id)
           FROM fused ORDER BY fscore DESC LIMIT {k}"
    );

    let rows: Vec<(i64, f64, f64, f64, JsonB)> = Spi::connect(|client| {
        client
            .select(&sql, None, &[query.into()])
            .unwrap_or_else(|e| error!("hybrid search failed: {e}"))
            .filter_map(|r| {
                Some((
                    r.get::<i64>(1).ok()??,
                    r.get::<f64>(2).ok()??,
                    r.get::<f64>(3).ok()??,
                    r.get::<f64>(4).ok()??,
                    r.get::<JsonB>(5).ok()?.unwrap_or(JsonB(json!({}))),
                ))
            })
            .collect()
    });
    TableIterator::new(rows)
}

/// Embeddings invalidated by a change to their source property (FR-022/023).
#[pg_extern(stable)]
fn og_stale_embeddings(graph: &str) -> TableIterator<
    'static,
    (name!(entity_id, i64), name!(type_name, String), name!(prop, String)),
> {
    let gid = types::graph_id(graph);
    let specs: Vec<(i32, String, String, String)> = Spi::connect(|client| {
        client
            .select(
                "SELECT e.type_id, t.name, e.prop, e.source_prop
                   FROM og_catalog.embedding e JOIN og_catalog.type t ON t.type_id = e.type_id
                  WHERE t.graph_id = $1 AND e.source_prop IS NOT NULL",
                None,
                &[gid.into()],
            )
            .expect("embedding scan failed")
            .filter_map(|r| {
                Some((
                    r.get::<i32>(1).ok()??,
                    r.get::<String>(2).ok()??,
                    r.get::<String>(3).ok()??,
                    r.get::<String>(4).ok()??,
                ))
            })
            .collect()
    });

    let mut out = Vec::new();
    for (tid, tname, prop, source) in specs {
        let ecol = types::column_name(&prop);
        let scol = types::column_name(&source);
        for sub in labeling::og_subtypes(tid) {
            let Some(table) = types::storage_table(sub) else { continue };
            let ids: Vec<i64> = Spi::connect(|client| {
                client
                    .select(
                        &format!(
                            "SELECT x.id FROM {table} x
                              LEFT JOIN og_data.og_embedding_state s
                                ON s.entity_id = x.id AND s.prop = '{prop}'
                              WHERE x.{scol} IS NOT NULL
                                AND (x.{ecol} IS NULL
                                     OR s.source_hash IS DISTINCT FROM md5(x.{scol}::text))"
                        ),
                        None,
                        &[],
                    )
                    .map(|rows| rows.filter_map(|r| r.get::<i64>(1).ok().flatten()).collect())
                    .unwrap_or_default()
            });
            for id in ids {
                out.push((id, tname.clone(), prop.clone()));
            }
        }
    }
    TableIterator::new(out)
}

/// Record that an embedding is current for the given source value.
#[pg_extern]
fn og_mark_embedded(entity_id: i64, prop: &str) {
    let tid = crate::id::id_type(entity_id);
    let source: Option<String> = crate::spiu::one::<String>(
        "SELECT source_prop FROM og_catalog.embedding
          WHERE type_id = ANY($1) AND prop = $2 LIMIT 1",
        &[labeling::og_supertypes(tid).into(), prop.into()],
    )
    .ok()
    .flatten();
    let Some(source) = source else { return };
    let Some(table) = types::storage_table(tid) else { return };
    let scol = types::column_name(&source);
    Spi::run_with_args(
        &format!(
            "INSERT INTO og_data.og_embedding_state (entity_id, prop, source_hash, embedded_at)
             SELECT $1, $2, md5(x.{scol}::text), now() FROM {table} x WHERE x.id = $1
             ON CONFLICT (entity_id, prop) DO UPDATE
                SET source_hash = EXCLUDED.source_hash, embedded_at = now()"
        ),
        &[entity_id.into(), prop.into()],
    )
    .expect("embedding state update failed");
}

#[pg_extern(stable, strict)]
fn og_embedding_stats(graph: &str) -> JsonB {
    let gid = types::graph_id(graph);
    let items: Vec<Value> = Spi::connect(|client| {
        client
            .select(
                "SELECT t.name, e.prop, e.dims, e.metric, e.source_prop
                   FROM og_catalog.embedding e JOIN og_catalog.type t ON t.type_id = e.type_id
                  WHERE t.graph_id = $1 ORDER BY t.name, e.prop",
                None,
                &[gid.into()],
            )
            .expect("embedding stats failed")
            .map(|r| {
                json!({
                    "type": r.get::<String>(1).unwrap(),
                    "property": r.get::<String>(2).unwrap(),
                    "dims": r.get::<i32>(3).unwrap(),
                    "metric": r.get::<String>(4).unwrap(),
                    "source_property": r.get::<String>(5).unwrap(),
                })
            })
            .collect()
    });
    JsonB(json!({ "graph": graph, "embeddings": items }))
}

/// Brute-force ground truth for measuring ANN recall — spec 004 FR-028.
#[pg_extern]
fn og_vector_search_exact(
    graph: &str,
    type_name: &str,
    prop: &str,
    query: &str,
    k: default!(i32, 10),
) -> TableIterator<'static, (name!(id, i64), name!(score, f64))> {
    let gid = types::graph_id(graph);
    let tid = types::type_id(gid, type_name);
    let (_, metric) = embedding_meta(tid, prop);
    let (op, _) = metric_op(&metric);
    let col = types::column_name(prop);
    let is_edge = types::type_kind(tid) == 'r';
    let view = crate::cypher::views::ensure_view(tid, is_edge);

    // Disabling index scans is the point: this is the reference implementation.
    Spi::run("SET LOCAL enable_indexscan = off").ok();
    let sql = format!(
        "SELECT v.id, (v.{col} {op} $1::vector)::float8 FROM {view} v
          WHERE v.{col} IS NOT NULL ORDER BY 2 LIMIT {k}"
    );
    let rows: Vec<(i64, f64)> = Spi::connect(|client| {
        client
            .select(&sql, None, &[query.into()])
            .unwrap_or_else(|e| error!("exact search failed: {e}"))
            .filter_map(|r| Some((r.get::<i64>(1).ok()??, r.get::<f64>(2).ok()??)))
            .collect()
    });
    Spi::run("SET LOCAL enable_indexscan = on").ok();
    TableIterator::new(rows)
}
