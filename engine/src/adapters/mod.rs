//! Semantic-web adapters — spec 006.
//!
//! One core model, adapters at the edge (Constitution VI). RDF is *mapped* onto
//! the type system of spec 002; it does not get a store of its own. Anything
//! that will not map is preserved verbatim in `og_triple_overflow` and reported,
//! because silently dropping a triple is the failure mode that destroys trust in
//! a round trip (FR-010).

pub mod rdf;

use crate::catalog::types;
use pgrx::prelude::*;
use pgrx::JsonB;
use serde_json::json;

/// Register a namespace prefix so IRIs can be written and read in short form.
#[pg_extern]
fn og_add_prefix(prefix: &str, iri: &str) {
    Spi::run_with_args(
        "INSERT INTO og_catalog.prefix (prefix, iri) VALUES ($1, $2)
         ON CONFLICT (prefix) DO UPDATE SET iri = EXCLUDED.iri",
        &[prefix.into(), iri.into()],
    )
    .expect("prefix registration failed");
}

/// Bind an existing type to an IRI, making it addressable from RDF.
#[pg_extern]
fn og_set_iri(graph: &str, type_name: &str, iri: &str) {
    let gid = types::graph_id(graph);
    let tid = types::type_id(gid, type_name);
    Spi::run_with_args(
        "UPDATE og_catalog.type SET iri = $2 WHERE type_id = $1",
        &[tid.into(), iri.into()],
    )
    .expect("iri update failed");
}

/// Load RDF (Turtle / N-Triples) into a graph — spec 006 FR-004, FR-011..FR-015.
#[pg_extern]
fn og_load_rdf(graph: &str, document: &str, format: default!(&str, "'turtle'")) -> JsonB {
    let report = rdf::load(graph, document, format);
    JsonB(json!(report))
}

/// Serialise a graph back to RDF — spec 006 FR-005.
#[pg_extern(stable)]
fn og_dump_rdf(graph: &str, format: default!(&str, "'turtle'")) -> String {
    rdf::dump(graph, format)
}

/// What did the last load fail to map? — spec 006 FR-010, FR-015.
#[pg_extern(stable)]
fn og_mapping_report(graph: &str) -> JsonB {
    let gid = types::graph_id(graph);
    let unmapped: Vec<serde_json::Value> = Spi::connect(|client| {
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
    });
    let total = crate::spiu::one::<i64>(
        "SELECT count(*) FROM og_data.og_triple_overflow WHERE graph_id = $1",
        &[gid.into()],
    )
    .ok()
    .flatten()
    .unwrap_or(0);

    JsonB(json!({
        "graph": graph,
        "unmapped_triples": total,
        "sample": unmapped,
        "note": "unmapped triples are preserved verbatim and re-emitted on export",
    }))
}
