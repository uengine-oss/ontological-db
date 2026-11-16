//! Graph storage — spec 001: the SQL functions every edition has.
//!
//! The five write functions below go through [`crate::layout::Layout`], so they
//! are the same in every edition; what they reach in v1 is
//! `crate::store_v1::storage`. Traversal (`og_reach`, the CSR) and the storage
//! statistics are algorithms over what the layout reads (P1 K5), so they stay
//! here too (P1 K6).

pub mod stats;
pub mod traverse;

use crate::catalog::types;
use pgrx::prelude::*;
use pgrx::JsonB;

// --------------------------------------------------------------------------
// Nodes
// --------------------------------------------------------------------------

#[pg_extern]
fn og_create_node(graph: &str, type_name: &str, props: default!(JsonB, "'{}'")) -> i64 {
    let gid = types::graph_id(graph);
    let tid = types::type_id(gid, type_name);
    crate::layout::active().create_node(gid, tid, type_name, props.0)
}

#[pg_extern]
fn og_set_node_props(id: i64, props: JsonB) {
    crate::layout::active().set_node_props(id, props.0);
}

/// Delete a node and every incident edge (DETACH DELETE semantics).
#[pg_extern]
fn og_delete_node(id: i64) -> i64 {
    crate::layout::active().delete_node(id)
}

// --------------------------------------------------------------------------
// Edges
// --------------------------------------------------------------------------

#[pg_extern]
fn og_create_edge(
    graph: &str,
    rel_type: &str,
    src: i64,
    dst: i64,
    props: default!(JsonB, "'{}'"),
) -> i64 {
    let gid = types::graph_id(graph);
    let tid = types::type_id(gid, rel_type);
    crate::layout::active().create_edge(gid, tid, rel_type, src, dst, props.0)
}

#[pg_extern]
fn og_delete_edge(id: i64) -> i64 {
    crate::layout::active().delete_edge(id)
}
