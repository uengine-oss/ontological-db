//! Neo4j compatibility surface.
//!
//! Cypher written against Neo4j does not only use the query language: it
//! creates indexes by name, calls `db.*` procedures, and reaches for a handful
//! of APOC helpers. An application cannot be moved here by rewriting its URI if
//! any of that is missing, so this module supplies it — under the **original
//! names**, mapped onto the native surface underneath.
//!
//! Nothing here adds semantics the engine lacks. `db.index.vector.queryNodes`
//! is `og_vector_search` reached by its Neo4j name; a `CREATE CONSTRAINT` is
//! `og_add_property(..., is_key := true)`. Where an equivalent genuinely does
//! not exist it is refused by name rather than approximated silently — except
//! full-text search, whose difference is documented in `docs/cypher.md` because
//! an approximation there is more useful than a refusal.

pub mod ddl;
pub mod genai;
pub mod meta;
pub mod procs;
