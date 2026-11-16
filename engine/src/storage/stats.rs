//! Storage statistics — spec 001 FR-015, FR-019.
//!
//! Two audiences: the Cypher planner (cardinality & selectivity) and operators /
//! agents (fragmentation, degree distribution, supernodes).
//!
//! The SQL functions are the same whatever the storage layout; what they read
//! and what they can report is the layout's ([`Layout::graph_stats`] and the
//! rest of the trait's "traversal and storage upkeep" methods). v1's queries —
//! the registries, the adjacency segments — are in `store_v1/layout.rs`.
//!
//! [`Layout::graph_stats`]: crate::layout::Layout::graph_stats

use pgrx::prelude::*;
use pgrx::JsonB;
use serde_json::json;

/// Per-type instance counts and degree distribution for one graph.
#[pg_extern(stable, strict)]
fn og_graph_stats(graph: &str) -> JsonB {
    let gid = crate::catalog::types::graph_id(graph);
    JsonB(crate::layout::active().graph_stats(gid, graph))
}

/// Degree histogram — used to spot supernodes before they hurt (FR-020).
#[pg_extern(stable, strict)]
fn og_degree_distribution(graph: &str) -> JsonB {
    let gid = crate::catalog::types::graph_id(graph);
    let buckets = crate::layout::active().degree_distribution(gid);
    JsonB(json!({ "graph": graph, "buckets": buckets }))
}

/// Online reorganisation — spec 001 FR-018.
///
/// Repacks fragmented storage for a graph without blocking readers. In v1 that
/// is the adjacency segments, each node's rewritten in one small
/// transaction-local update.
#[pg_extern]
fn og_reorganize(graph: &str) -> i64 {
    let gid = crate::catalog::types::graph_id(graph);
    crate::layout::active().reorganize(gid)
}

/// Structural integrity checker — spec 009 FR-015.
///
/// Cross-checks what the layout keeps in more than one place (v1: registry,
/// typed tables and adjacency segments). Returns one row per inconsistency; an
/// empty result is the pass condition.
#[pg_extern(stable)]
fn og_check_integrity() -> TableIterator<
    'static,
    (
        name!(kind, String),
        name!(entity_id, Option<i64>),
        name!(detail, String),
    ),
> {
    TableIterator::new(crate::layout::active().check_integrity())
}
