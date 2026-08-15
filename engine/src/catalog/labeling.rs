//! Interval (nested-set) labelling of the type hierarchy — spec 002 FR-009..FR-014.
//!
//! This module is the single reason `MATCH (v:Vehicle)` does not degrade as the
//! ontology grows.  Every type carries one `(lft, rgt)` interval per path from a
//! root, so the question *"is X a subtype of Y?"* reduces to
//!
//! ```text
//!     Y.lft <= X.lft  AND  X.rgt <= Y.rgt
//! ```
//!
//! one indexed range comparison — never a recursive CTE (which is what spec 002
//! FR-010 forbids and SC-003 asserts against by inspecting EXPLAIN output).
//!
//! Labels are handed out with a gap of [`GAP`] so that inserting a type in the
//! middle of a hierarchy usually consumes free space instead of renumbering the
//! world (FR-013).

use pgrx::prelude::*;
use std::collections::{HashMap, HashSet};

/// Spacing between successive labels. Inserting a type between a parent and its
/// child costs nothing until 1024 insertions have happened at the same spot.
pub const GAP: i64 = 1024;

/// A single node of the inheritance DAG as loaded from the catalog.
struct TypeNode {
    id: i32,
    children: Vec<i32>,
    parents: Vec<i32>,
}

fn load_dag(graph_id: i32) -> HashMap<i32, TypeNode> {
    let mut dag: HashMap<i32, TypeNode> = HashMap::new();

    Spi::connect(|client| {
        let rows = client
            .select(
                "SELECT type_id FROM og_catalog.type WHERE graph_id = $1",
                None,
                &[graph_id.into()],
            )
            .expect("failed to load types");
        for row in rows {
            let id: i32 = row.get(1).unwrap().unwrap();
            dag.insert(id, TypeNode { id, children: vec![], parents: vec![] });
        }

        let rows = client
            .select(
                "SELECT tp.type_id, tp.parent_id
                   FROM og_catalog.type_parent tp
                   JOIN og_catalog.type t ON t.type_id = tp.type_id
                  WHERE t.graph_id = $1",
                None,
                &[graph_id.into()],
            )
            .expect("failed to load type_parent");
        for row in rows {
            let child: i32 = row.get(1).unwrap().unwrap();
            let parent: i32 = row.get(2).unwrap().unwrap();
            if let Some(p) = dag.get_mut(&parent) {
                p.children.push(child);
            }
            if let Some(c) = dag.get_mut(&child) {
                c.parents.push(parent);
            }
        }
    });

    dag
}

/// Depth-first assignment of `(lft, rgt)` intervals with [`GAP`] spacing.
///
/// A type reachable through several parents receives one label row per path,
/// which is what keeps multiple inheritance answerable by range comparison
/// (FR-012).
fn assign(
    dag: &HashMap<i32, TypeNode>,
    id: i32,
    depth: i32,
    cursor: &mut i64,
    path_counter: &mut HashMap<i32, i32>,
    out: &mut Vec<(i32, i32, i64, i64, i32)>,
    stack: &mut Vec<i32>,
) {
    if stack.contains(&id) {
        error!("inheritance cycle detected at type {id}");
    }
    stack.push(id);

    let lft = *cursor;
    *cursor += GAP;

    let node = &dag[&id];
    let mut children = node.children.clone();
    children.sort_unstable();
    for child in children {
        assign(dag, child, depth + 1, cursor, path_counter, out, stack);
    }

    let rgt = *cursor;
    *cursor += GAP;

    let path_id = path_counter.entry(id).or_insert(0);
    out.push((id, *path_id, lft, rgt, depth));
    *path_id += 1;

    stack.pop();
}

/// Recompute every interval label for `graph_id`.
///
/// Called on any structural change to the hierarchy. For large ontologies this
/// is the fallback path — [`insert_between`] handles the common case without
/// touching existing labels.
pub fn relabel_graph(graph_id: i32) {
    let dag = load_dag(graph_id);
    if dag.is_empty() {
        return;
    }

    // Roots = types with no parent inside this graph.
    let mut roots: Vec<i32> = dag.values().filter(|n| n.parents.is_empty()).map(|n| n.id).collect();
    roots.sort_unstable();

    // Detect nodes unreachable from any root: that only happens on a cycle.
    let mut reachable: HashSet<i32> = HashSet::new();
    let mut queue: Vec<i32> = roots.clone();
    let mut guard = 0usize;
    while let Some(id) = queue.pop() {
        guard += 1;
        if guard > dag.len() * 64 {
            error!("inheritance cycle detected while walking the type hierarchy");
        }
        if !reachable.insert(id) {
            continue;
        }
        queue.extend(dag[&id].children.iter().copied());
    }
    if reachable.len() != dag.len() {
        let orphans: Vec<i32> = dag.keys().filter(|k| !reachable.contains(k)).copied().collect();
        error!("inheritance cycle detected involving type(s) {orphans:?}");
    }

    let mut cursor: i64 = 1;
    let mut path_counter: HashMap<i32, i32> = HashMap::new();
    let mut out: Vec<(i32, i32, i64, i64, i32)> = Vec::with_capacity(dag.len());
    let mut stack: Vec<i32> = Vec::new();
    for root in roots {
        assign(&dag, root, 0, &mut cursor, &mut path_counter, &mut out, &mut stack);
    }

    Spi::run_with_args(
        "DELETE FROM og_catalog.type_label WHERE graph_id = $1",
        &[graph_id.into()],
    )
    .expect("failed to clear labels");

    for (type_id, path_id, lft, rgt, depth) in out {
        Spi::run_with_args(
            "INSERT INTO og_catalog.type_label (type_id, path_id, graph_id, lft, rgt, depth)
             VALUES ($1, $2, $3, $4, $5, $6)",
            &[type_id.into(), path_id.into(), graph_id.into(), lft.into(), rgt.into(), depth.into()],
        )
        .expect("failed to insert label");
    }

    bump_schema_version(graph_id, "relabel");
}

pub fn bump_schema_version(graph_id: i32, description: &str) {
    // Generated per-type union views encode the descendant set, so any schema
    // change invalidates them (spec 003 / cypher::views).
    crate::cypher::views::drop_all_views();
    Spi::run_with_args(
        "INSERT INTO og_catalog.schema_version (version, graph_id, description)
         VALUES (nextval('og_catalog.schema_version_seq'), $1, $2)",
        &[graph_id.into(), description.into()],
    )
    .expect("failed to bump schema version");
}

// --------------------------------------------------------------------------
// SQL surface — these are what the Cypher compiler emits
// --------------------------------------------------------------------------

/// All type ids that are `type_id` or any of its descendants.
///
/// One indexed range scan on `og_catalog.type_label`. This is the expansion of
/// a label predicate in a compiled Cypher plan.
#[pg_extern(stable, parallel_safe, strict)]
pub fn og_subtypes(type_id: i32) -> Vec<i32> {
    Spi::connect(|client| {
        let rows = client
            .select(
                "SELECT DISTINCT d.type_id
                   FROM og_catalog.type_label a
                   JOIN og_catalog.type_label d
                     ON d.graph_id = a.graph_id
                    AND d.lft >= a.lft AND d.rgt <= a.rgt
                  WHERE a.type_id = $1",
                None,
                &[type_id.into()],
            )
            .expect("subtype lookup failed");
        rows.map(|r| r.get::<i32>(1).unwrap().unwrap()).collect()
    })
}

/// All type ids that are `type_id` or any of its ancestors.
#[pg_extern(stable, parallel_safe, strict)]
pub fn og_supertypes(type_id: i32) -> Vec<i32> {
    Spi::connect(|client| {
        let rows = client
            .select(
                "SELECT DISTINCT a.type_id
                   FROM og_catalog.type_label d
                   JOIN og_catalog.type_label a
                     ON a.graph_id = d.graph_id
                    AND d.lft >= a.lft AND d.rgt <= a.rgt
                  WHERE d.type_id = $1",
                None,
                &[type_id.into()],
            )
            .expect("supertype lookup failed");
        rows.map(|r| r.get::<i32>(1).unwrap().unwrap()).collect()
    })
}

/// Constant-time subtype test: `sub ⊑ sup`.
#[pg_extern(stable, parallel_safe, strict)]
pub fn og_is_subtype(sub: i32, sup: i32) -> bool {
    crate::spiu::one::<bool>(
        "SELECT EXISTS (
           SELECT 1 FROM og_catalog.type_label d, og_catalog.type_label a
            WHERE d.type_id = $1 AND a.type_id = $2
              AND a.graph_id = d.graph_id
              AND d.lft >= a.lft AND d.rgt <= a.rgt)",
        &[sub.into(), sup.into()],
    )
    .expect("subtype test failed")
    .unwrap_or(false)
}

/// Re-run labelling by hand (diagnostics / repair).
#[pg_extern(strict)]
fn og_relabel(graph_id: i32) {
    relabel_graph(graph_id);
}
