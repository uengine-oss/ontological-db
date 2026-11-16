//! v1's answers to the schema questions the agent interface and Neo4j's
//! `apoc.meta.schema()` ask of the data (P1 K6): how many instances each type
//! has (v1 counts registry rows, `og_data.og_node` / `og_edge`), and which
//! endpoint types a relationship type's edges actually join (v1 reads them off
//! the edge ids, whose type is a bit field). The statements are the ones
//! `crate::agent::og_schema` and `crate::compat::meta` ran, text and order.

use pgrx::prelude::*;

/// Every type of graph `gid` with its instance count, most instances first —
/// `og_schema`'s scan: `(type_id, name, kind, is_abstract, instances)`.
pub fn schema_types(gid: i32) -> Vec<(i32, String, String, bool, i64)> {
    Spi::connect(|client| {
        client
            .select(
                "SELECT t.type_id, t.name, t.kind::text, t.is_abstract,
                        COALESCE((SELECT count(*) FROM og_data.og_node n WHERE n.type_id = t.type_id), 0)
                      + COALESCE((SELECT count(*) FROM og_data.og_edge e WHERE e.type_id = t.type_id), 0)
                   FROM og_catalog.type t WHERE t.graph_id = $1
                  ORDER BY 5 DESC, t.name",
                None,
                &[gid.into()],
            )
            .expect("schema scan failed")
            .filter_map(|r| {
                Some((
                    r.get::<i32>(1).ok()??,
                    r.get::<String>(2).ok()??,
                    r.get::<String>(3).ok()??,
                    r.get::<bool>(4).ok()?.unwrap_or(false),
                    r.get::<i64>(5).ok()?.unwrap_or(0),
                ))
            })
            .collect()
    })
}

/// Every entity and relation type of graph `gid` with its instance count, by
/// name — `og_apoc_meta_schema`'s scan: `(type_id, name, kind, count)`.
pub fn apoc_types(gid: i32) -> Vec<(i32, String, String, i64)> {
    Spi::connect(|client| {
        client
            .select(
                "SELECT t.type_id, t.name, t.kind::text,
                        COALESCE((SELECT count(*) FROM og_data.og_node n WHERE n.type_id = t.type_id), 0)
                      + COALESCE((SELECT count(*) FROM og_data.og_edge e WHERE e.type_id = t.type_id), 0)
                   FROM og_catalog.type t
                  WHERE t.graph_id = $1 AND t.kind IN ('e', 'r')
                  ORDER BY t.name",
                None,
                &[gid.into()],
            )
            .expect("meta scan failed")
            .filter_map(|r| {
                Some((
                    r.get::<i32>(1).ok()??,
                    r.get::<String>(2).ok()??,
                    r.get::<String>(3).ok()??,
                    r.get::<i64>(4).ok()?.unwrap_or(0),
                ))
            })
            .collect()
    })
}

/// The endpoint types of up to `sample` edges of relationship type `tid`,
/// distinct — what `compat::meta` reports for a relationship type that
/// declares no roles (APOC's sampling, see there).
///
/// The type of a node is a bit field of its id, so this needs no join to the
/// node table — and `og_edge_type_idx` leads with `type_id`, so the sample is a
/// bounded index scan rather than a scan of every edge.
pub fn observed_endpoints(tid: i32, sample: i32) -> Vec<(i32, i32)> {
    Spi::connect(|client| {
        client
            .select(
                "SELECT DISTINCT og_id_type(e.src), og_id_type(e.dst)
                   FROM (SELECT src, dst FROM og_data.og_edge
                          WHERE type_id = $1 ORDER BY id LIMIT $2) e",
                None,
                &[tid.into(), (sample.max(1) as i64).into()],
            )
            .map(|rows| {
                rows.filter_map(|r| Some((r.get::<i32>(1).ok()??, r.get::<i32>(2).ok()??)))
                    .collect()
            })
            .unwrap_or_default()
    })
}
