//! PostgreSQL / Supabase interoperability — spec 005.
//!
//! The load-bearing observation: a compiled Cypher query reads ordinary tables.
//! So row-level security applied to those tables applies **mid-traversal**, with
//! no enforcement code of our own — a node the caller cannot see simply fails to
//! join, and every path through it disappears (FR-013). A forked database would
//! have had to build that; an extension inherits it.
//!
//! What stays here is the entry point every edition has (`og_cypher_json`).
//! Row-level security on the type tables, mapping a relational table onto a
//! type, materialising it and the report of all that are v1's storage and live
//! in `crate::store_v1::interop` (P1 K6).

use pgrx::prelude::*;
use pgrx::JsonB;
use serde_json::Value;

/// PostgREST / supabase-js entry point: one call, one JSON array.
#[pg_extern]
fn og_cypher_json(graph: &str, query: &str, params: default!(JsonB, "'{}'")) -> JsonB {
    let rows: Vec<Value> = Spi::connect(|client| {
        client
            .select(
                &format!("SELECT coalesce(jsonb_agg(r), '[]'::jsonb) FROM {}($1,$2,$3) r", crate::spiu::fnq("og_cypher")),
                None,
                &[graph.into(), query.into(), params.into()],
            )
            .unwrap_or_else(|e| error!("{e}"))
            .next()
            .and_then(|r| r.get::<JsonB>(1).ok().flatten())
            .map(|j| match j.0 {
                Value::Array(a) => a,
                other => vec![other],
            })
            .unwrap_or_default()
    });
    JsonB(Value::Array(rows))
}
