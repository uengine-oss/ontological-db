//! # Ontological
//!
//! A Cypher-native ontology graph database that lives inside PostgreSQL.
//!
//! Layout mirrors the specs:
//!
//! | module     | spec |
//! |------------|------|
//! | [`id`]     | 001 — identifier encoding |
//! | `storage`  | 001 — adjacency segments, type tables |
//! | `catalog`  | 002 — type system, interval labels, roles, constraints |
//! | `cypher`   | 003 — parser, planner, executor |
//! | `vector`   | 004 — node/edge/path embeddings |
//! | `interop`  | 005 — SQL bridge, relational mapping |
//! | `adapters` | 006 — RDF / SPARQL |
//! | `agent`    | 008 — introspection, provenance, guardrails |

use pgrx::prelude::*;

::pgrx::pg_module_magic!(name, version);

extension_sql_file!("../sql/bootstrap.sql", name = "bootstrap", bootstrap);
extension_sql_file!("../sql/access.sql", name = "access", finalize);

pub mod adapters;
pub mod agent;
pub mod catalog;
pub mod cypher;
pub mod spiu;
pub mod id;
pub mod interop;
pub mod storage;
pub mod typeql;
pub mod vector;

/// Extension version, for clients that need to gate on capabilities.
#[pg_extern(immutable, parallel_safe)]
fn ontological_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

// In-database behaviour is covered by the SQL regression suite in
// `engine/tests/sql/`, run with `tests/run.sh` against a live server. Pure Rust
// units (lexer, parser, identifier encoding, RDF parsing) live beside their
// modules and run under `cargo test`.
