//! v1's RDF tables (spec 006, P1 K6): the IRI of each loaded instance
//! (`og_data.og_iri`), the triples a load could not map (`og_data.og_triple_overflow`,
//! kept verbatim and re-emitted on export, FR-010), and the export's scans of
//! v1's registries. `crate::adapters` parses, maps and serialises; the
//! statements here are the ones it ran, text and order.

use pgrx::prelude::*;
use serde_json::{json, Value};

/// Remember the IRI an instance was loaded from (pass 4 of `rdf::load`); one
/// IRI names one instance, the first to claim it.
pub fn record_iri(id: i64, iri: &str) {
    Spi::run_with_args(
        "INSERT INTO og_data.og_iri (entity_id, iri) VALUES ($1,$2)
                 ON CONFLICT (iri) DO NOTHING",
        &[id.into(), iri.into()],
    )
    .ok();
}

/// Keep a triple the load could not map, verbatim, with why.
pub fn record_overflow(gid: i32, subject: String, predicate: String, object: String, reason: &str) {
    Spi::run_with_args(
        "INSERT INTO og_data.og_triple_overflow (graph_id, subject, predicate, object, reason)
         VALUES ($1,$2,$3,$4,$5)",
        &[
            gid.into(),
            subject.into(),
            predicate.into(),
            object.into(),
            reason.into(),
        ],
    )
    .ok();
}

/// Every node of graph `gid`: `(id, type IRI, the node as JSON)` — the ABox of `rdf::dump`.
pub fn dump_nodes(gid: i32) -> Vec<(i64, String, Value)> {
    Spi::connect(|client| {
        client
            .select(
                "SELECT n.id, COALESCE(t.iri, 'urn:og:' || t.name), og_node_json(n.id)
                   FROM og_data.og_node n
                   JOIN og_catalog.type t ON t.type_id = n.type_id
                  WHERE t.graph_id = $1",
                None,
                &[gid.into()],
            )
            .expect("node dump failed")
            .filter_map(|r| {
                Some((
                    r.get::<i64>(1).ok()??,
                    r.get::<String>(2).ok()??,
                    r.get::<pgrx::JsonB>(3).ok()??.0,
                ))
            })
            .collect()
    })
}

/// Every relationship of graph `gid` as `(subject IRI, predicate IRI, object IRI)`.
pub fn dump_edges(gid: i32) -> Vec<(String, String, String)> {
    Spi::connect(|client| {
        client
            .select(
                "SELECT COALESCE(si.iri, 'urn:og:node:' || e.src),
                        COALESCE(t.iri, 'urn:og:' || t.name),
                        COALESCE(di.iri, 'urn:og:node:' || e.dst)
                   FROM og_data.og_edge e
                   JOIN og_catalog.type t ON t.type_id = e.type_id
                   LEFT JOIN og_data.og_iri si ON si.entity_id = e.src
                   LEFT JOIN og_data.og_iri di ON di.entity_id = e.dst
                  WHERE t.graph_id = $1",
                None,
                &[gid.into()],
            )
            .expect("edge dump failed")
            .filter_map(|r| {
                Some((
                    r.get::<String>(1).ok()??,
                    r.get::<String>(2).ok()??,
                    r.get::<String>(3).ok()??,
                ))
            })
            .collect()
    })
}

/// The triples of graph `gid` a load could not map, for the export to re-emit.
pub fn dump_overflow(gid: i32) -> Vec<(String, String, String)> {
    Spi::connect(|client| {
        client
            .select(
                "SELECT subject, predicate, object FROM og_data.og_triple_overflow
                  WHERE graph_id = $1",
                None,
                &[gid.into()],
            )
            .map(|rows| {
                rows.filter_map(|r| {
                    Some((
                        r.get::<String>(1).ok()??,
                        r.get::<String>(2).ok()??,
                        r.get::<String>(3).ok()??,
                    ))
                })
                .collect()
            })
            .unwrap_or_default()
    })
}

/// Up to 200 unmapped triples of graph `gid`, oldest first — `og_mapping_report`'s sample.
pub fn overflow_sample(gid: i32) -> Vec<Value> {
    Spi::connect(|client| {
        client
            .select(
                "SELECT subject, predicate, object, reason FROM og_data.og_triple_overflow
                  WHERE graph_id = $1 ORDER BY id LIMIT 200",
                None,
                &[gid.into()],
            )
            .expect("overflow scan failed")
            .map(|r| {
                json!({
                    "subject": r.get::<String>(1).unwrap(),
                    "predicate": r.get::<String>(2).unwrap(),
                    "object": r.get::<String>(3).unwrap(),
                    "reason": r.get::<String>(4).unwrap(),
                })
            })
            .collect()
    })
}

/// How many triples of graph `gid` a load could not map.
pub fn overflow_count(gid: i32) -> i64 {
    crate::spiu::one::<i64>(
        "SELECT count(*) FROM og_data.og_triple_overflow WHERE graph_id = $1",
        &[gid.into()],
    )
    .ok()
    .flatten()
    .unwrap_or(0)
}
