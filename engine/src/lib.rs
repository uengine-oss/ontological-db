//! # Ontological
//!
//! A Cypher-native ontology graph database that lives inside PostgreSQL.
//!
//! Layout mirrors the specs:
//!
//! | module     | spec |
//! |------------|------|
//! | [`id`]     | 001 — identifier encoding |
//! | `storage`  | 001 — the write and traversal SQL functions every edition has |
//! | `catalog`  | 002 — type system, interval labels, roles, constraints |
//! | `cypher`   | 003 — parser, planner, executor |
//! | `interop`  | 005 — SQL bridge |
//! | `adapters` | 006 — RDF / SPARQL |
//! | `agent`    | 008 — introspection, guardrails |
//! | `compat`   | Neo4j surface: named indexes, `db.*` / `apoc.*` procedures |
//! | `layout`   | the storage-layout seam (`DESIGN.md` 5.1) |
//! | `store_v1` | v1's storage: adjacency segments, type tables, views, history, security, embeddings (P1 K6) |

use pgrx::prelude::*;

::pgrx::pg_module_magic!(name, version);

extension_sql_file!("../sql/bootstrap.sql", name = "bootstrap", bootstrap);
extension_sql_file!("../sql/release.sql", name = "release", requires = ["bootstrap"]);
extension_sql_file!("../sql/access.sql", name = "access", finalize);

pub mod adapters;
pub mod agent;
pub mod catalog;
pub mod compat;
pub mod cypher;
pub mod fabric;
pub mod spiu;
pub mod id;
pub mod interop;
pub mod layout;
pub mod stats;
pub mod storage;
pub mod store_v1;
pub mod typeql;

/// Extension version, for clients that need to gate on capabilities.
#[pg_extern(immutable, parallel_safe)]
fn ontological_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

// In-database behaviour is covered by the SQL regression suite in
// `tests/{common,v1}/sql/`, run with `tests/run.sh` against a live server. Pure Rust
// units (lexer, parser, identifier encoding, RDF parsing) live beside their
// modules and run under `cargo test`.
