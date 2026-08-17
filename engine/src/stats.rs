//! What a write changed, counted as Neo4j counts it.
//!
//! A Cypher write here is executed clause by clause from Rust rather than as
//! one SQL statement, so there is no single row count to report. The counters
//! are therefore taken where the change actually happens — in `storage` — which
//! is also the only place that sees `MERGE` deciding to create rather than
//! match.
//!
//! The state is per-backend and reset at the start of every `og_cypher()` call.
//! That is sound because a PostgreSQL backend serves one connection and runs one
//! statement at a time: there is no second writer to interleave with. It is
//! *not* a transaction log — a rolled-back statement leaves its counts behind,
//! and the next call clears them.
//!
//! The names are Neo4j's, hyphens included, because a driver turns them
//! straight into `ResultSummary.counters` fields.

use serde_json::{json, Value};
use std::cell::Cell;

macro_rules! counters {
    ($($field:ident => $key:literal),* $(,)?) => {
        thread_local! {
            $(static $field: Cell<i64> = const { Cell::new(0) };)*
        }

        /// Start a fresh count. Called once per `og_cypher()`.
        pub fn reset() {
            $($field.with(|c| c.set(0));)*
        }

        /// The counts since the last `reset()`, in Neo4j's spelling.
        ///
        /// `contains-updates` is what a driver checks before showing any of the
        /// rest, so it is derived here rather than left to the caller.
        pub fn snapshot() -> Value {
            let counted: [(&'static str, i64); [$($key),*].len()] =
                [$(($key, $field.with(Cell::get))),*];
            let total: i64 = counted.iter().map(|(_, n)| *n).sum();
            let mut out = serde_json::Map::new();
            for (key, n) in counted {
                out.insert(key.to_string(), json!(n));
            }
            out.insert("contains-updates".into(), json!(total > 0));
            Value::Object(out)
        }
    };
}

counters! {
    NODES_CREATED         => "nodes-created",
    NODES_DELETED         => "nodes-deleted",
    RELATIONSHIPS_CREATED => "relationships-created",
    RELATIONSHIPS_DELETED => "relationships-deleted",
    PROPERTIES_SET        => "properties-set",
    LABELS_ADDED          => "labels-added",
    INDEXES_ADDED         => "indexes-added",
    INDEXES_REMOVED       => "indexes-removed",
    CONSTRAINTS_ADDED     => "constraints-added",
    CONSTRAINTS_REMOVED   => "constraints-removed",
}

fn bump(counter: &'static std::thread::LocalKey<Cell<i64>>, by: i64) {
    counter.with(|c| c.set(c.get() + by));
}

/// A node is created with exactly one label here, so the two move together.
pub fn node_created() {
    bump(&NODES_CREATED, 1);
    bump(&LABELS_ADDED, 1);
}

pub fn node_deleted() {
    bump(&NODES_DELETED, 1);
}

pub fn relationship_created() {
    bump(&RELATIONSHIPS_CREATED, 1);
}

pub fn relationship_deleted() {
    bump(&RELATIONSHIPS_DELETED, 1);
}

/// Counts the properties in one write, not the call.
///
/// Neo4j counts a property *assignment*, so setting the same key twice counts
/// twice and `CREATE (n {a: 1, b: 2})` counts two. Anything that is not an
/// object carries no properties.
pub fn properties_set(props: &Value) {
    if let Value::Object(m) = props {
        bump(&PROPERTIES_SET, m.len() as i64);
    }
}

pub fn index_added() {
    bump(&INDEXES_ADDED, 1);
}

pub fn index_removed() {
    bump(&INDEXES_REMOVED, 1);
}

pub fn constraint_added() {
    bump(&CONSTRAINTS_ADDED, 1);
}

pub fn constraint_removed() {
    bump(&CONSTRAINTS_REMOVED, 1);
}
