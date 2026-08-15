//! Storage statistics — spec 001 FR-015, FR-019.
//!
//! Two audiences: the Cypher planner (cardinality & selectivity) and operators /
//! agents (fragmentation, degree distribution, supernodes).

use pgrx::prelude::*;
use pgrx::JsonB;
use serde_json::json;

/// Per-type instance counts and degree distribution for one graph.
#[pg_extern(stable, strict)]
fn og_graph_stats(graph: &str) -> JsonB {
    let gid = crate::catalog::types::graph_id(graph);

    let (nodes, edges) = crate::spiu::two::<i64, i64>(
        "SELECT (SELECT count(*) FROM og_data.og_node n
                   JOIN og_catalog.type t ON t.type_id = n.type_id WHERE t.graph_id = $1),
                (SELECT count(*) FROM og_data.og_edge e
                   JOIN og_catalog.type t ON t.type_id = e.type_id WHERE t.graph_id = $1)",
        &[gid.into()],
    )
    .expect("count failed");

    let types: Vec<serde_json::Value> = Spi::connect(|client| {
        client
            .select(
                "SELECT t.name, t.kind::text, t.is_abstract,
                        COALESCE((SELECT count(*) FROM og_data.og_node n WHERE n.type_id = t.type_id), 0)
                      + COALESCE((SELECT count(*) FROM og_data.og_edge e WHERE e.type_id = t.type_id), 0)
                   FROM og_catalog.type t WHERE t.graph_id = $1 ORDER BY t.name",
                None,
                &[gid.into()],
            )
            .expect("type stats failed")
            .map(|r| {
                json!({
                    "name": r.get::<String>(1).unwrap(),
                    "kind": r.get::<String>(2).unwrap(),
                    "abstract": r.get::<bool>(3).unwrap().unwrap_or(false),
                    "instances": r.get::<i64>(4).unwrap().unwrap_or(0),
                })
            })
            .collect()
    });

    let (segments, fill, supernodes) = Spi::connect(|client| {
        client
            .select(
                "SELECT count(*)::int8,
                        COALESCE(avg(n)::float8, 0),
                        count(*) FILTER (WHERE seq > 0)::int8
                   FROM og_data.og_adj",
                None,
                &[],
            )
            .expect("segment stats failed")
            .next()
            .map(|r| {
                (
                    r.get::<i64>(1).unwrap().unwrap_or(0),
                    r.get::<f64>(2).unwrap().unwrap_or(0.0),
                    r.get::<i64>(3).unwrap().unwrap_or(0),
                )
            })
            .unwrap_or((0, 0.0, 0))
    });

    let chunk = crate::storage::adjacency::CHUNK as f64;
    JsonB(json!({
        "graph": graph,
        "nodes": nodes.unwrap_or(0),
        "edges": edges.unwrap_or(0),
        "types": types,
        "adjacency": {
            "segments": segments,
            "avg_fill": fill,
            "chunk_size": chunk,
            // 1.0 = perfectly packed; lower means reorganisation will help.
            "packing_ratio": if segments > 0 { fill / chunk } else { 1.0 },
            "chunked_supernodes": supernodes,
        }
    }))
}

/// Degree histogram — used to spot supernodes before they hurt (FR-020).
#[pg_extern(stable, strict)]
fn og_degree_distribution(graph: &str) -> JsonB {
    let gid = crate::catalog::types::graph_id(graph);
    let buckets: Vec<serde_json::Value> = Spi::connect(|client| {
        client
            .select(
                "WITH d AS (
                     SELECT a.src, sum(a.n) AS deg
                       FROM og_data.og_adj a
                       JOIN og_data.og_node nd ON nd.id = a.src
                       JOIN og_catalog.type t ON t.type_id = nd.type_id AND t.graph_id = $1
                      WHERE a.dir = 'o'
                      GROUP BY a.src)
                 SELECT width_bucket(deg, 0, 1024, 8) AS b, count(*)::int8, max(deg)::int8
                   FROM d GROUP BY b ORDER BY b",
                None,
                &[gid.into()],
            )
            .expect("degree distribution failed")
            .map(|r| {
                json!({
                    "bucket": r.get::<i32>(1).unwrap(),
                    "nodes": r.get::<i64>(2).unwrap().unwrap_or(0),
                    "max_degree": r.get::<i64>(3).unwrap().unwrap_or(0),
                })
            })
            .collect()
    });
    JsonB(json!({ "graph": graph, "buckets": buckets }))
}

/// Online reorganisation — spec 001 FR-018.
///
/// Repacks fragmented adjacency segments for a graph without blocking readers:
/// each node's segments are rewritten in one small transaction-local update.
#[pg_extern]
fn og_reorganize(graph: &str) -> i64 {
    let gid = crate::catalog::types::graph_id(graph);
    let targets: Vec<(i64, i32, String)> = Spi::connect(|client| {
        client
            .select(
                "SELECT a.src, a.etype, a.dir::text
                   FROM og_data.og_adj a
                   JOIN og_data.og_node nd ON nd.id = a.src
                   JOIN og_catalog.type t ON t.type_id = nd.type_id AND t.graph_id = $1
                  GROUP BY a.src, a.etype, a.dir
                 HAVING count(*) > 1 AND sum(a.n) <= count(*) * $2 - $2",
                None,
                &[gid.into(), crate::storage::adjacency::CHUNK.into()],
            )
            .expect("reorg scan failed")
            .filter_map(|r| {
                Some((r.get::<i64>(1).ok()??, r.get::<i32>(2).ok()??, r.get::<String>(3).ok()??))
            })
            .collect()
    });

    let mut repacked = 0i64;
    for (src, etype, dir) in targets {
        Spi::run_with_args(
            "WITH flat AS (
                 SELECT u.nbr, u.eid,
                        ((row_number() OVER (ORDER BY a.seq)) - 1) / $4 AS newseq
                   FROM og_data.og_adj a, LATERAL unnest(a.nbr, a.eid) AS u(nbr, eid)
                  WHERE a.src = $1 AND a.etype = $2 AND a.dir = $3::text::\"char\"),
             packed AS (
                 SELECT newseq, count(*)::int4 AS n,
                        array_agg(nbr) AS nbr, array_agg(eid) AS eid
                   FROM flat GROUP BY newseq),
             wipe AS (
                 DELETE FROM og_data.og_adj
                  WHERE src = $1 AND etype = $2 AND dir = $3::text::\"char\")
             INSERT INTO og_data.og_adj (src, etype, dir, seq, n, nbr, eid)
             SELECT $1, $2, $3::text::\"char\", newseq::int4, n, nbr, eid FROM packed",
            &[src.into(), etype.into(), dir.into(), (crate::storage::adjacency::CHUNK as i64).into()],
        )
        .expect("repack failed");
        repacked += 1;
    }
    repacked
}

/// Structural integrity checker — spec 009 FR-015.
///
/// Cross-checks registry, typed tables and adjacency segments. Returns one row
/// per inconsistency; an empty result is the pass condition.
#[pg_extern(stable)]
fn og_check_integrity() -> TableIterator<
    'static,
    (
        name!(kind, String),
        name!(entity_id, Option<i64>),
        name!(detail, String),
    ),
> {
    let mut findings: Vec<(String, Option<i64>, String)> = Vec::new();

    Spi::connect(|client| {
        // 1. adjacency entries pointing at edges that no longer exist
        let rows = client
            .select(
                "SELECT DISTINCT e FROM og_data.og_adj a, LATERAL unnest(a.eid) AS e
                  WHERE NOT EXISTS (SELECT 1 FROM og_data.og_edge x WHERE x.id = e) LIMIT 100",
                None,
                &[],
            )
            .expect("integrity scan 1 failed");
        for r in rows {
            findings.push((
                "dangling_adjacency".into(),
                r.get::<i64>(1).ok().flatten(),
                "adjacency references a non-existent edge".into(),
            ));
        }

        // 2. edges whose adjacency entry is missing in either direction
        let rows = client
            .select(
                "SELECT e.id FROM og_data.og_edge e
                  WHERE NOT EXISTS (
                        SELECT 1 FROM og_data.og_adj a
                         WHERE a.src = e.src AND a.dir = 'o' AND e.id = ANY(a.eid))
                     OR NOT EXISTS (
                        SELECT 1 FROM og_data.og_adj a
                         WHERE a.src = e.dst AND a.dir = 'i' AND e.id = ANY(a.eid))
                  LIMIT 100",
                None,
                &[],
            )
            .expect("integrity scan 2 failed");
        for r in rows {
            findings.push((
                "missing_adjacency".into(),
                r.get::<i64>(1).ok().flatten(),
                "edge is not reachable from one of its endpoints".into(),
            ));
        }

        // 3. array length vs n
        let rows = client
            .select(
                "SELECT src FROM og_data.og_adj
                  WHERE n <> COALESCE(array_length(nbr,1),0)
                     OR COALESCE(array_length(nbr,1),0) <> COALESCE(array_length(eid,1),0)
                  LIMIT 100",
                None,
                &[],
            )
            .expect("integrity scan 3 failed");
        for r in rows {
            findings.push((
                "segment_length_mismatch".into(),
                r.get::<i64>(1).ok().flatten(),
                "segment count does not match array length".into(),
            ));
        }

        // 4. registry vs typed table
        let rows = client
            .select(
                "SELECT n.id FROM og_data.og_node n
                  WHERE NOT EXISTS (SELECT 1 FROM og_catalog.type t WHERE t.type_id = n.type_id)
                  LIMIT 100",
                None,
                &[],
            )
            .expect("integrity scan 4 failed");
        for r in rows {
            findings.push((
                "orphan_node".into(),
                r.get::<i64>(1).ok().flatten(),
                "node references an unknown type".into(),
            ));
        }
    });

    TableIterator::new(findings)
}
