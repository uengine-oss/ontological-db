//! History and provenance — spec 008 FR-015, FR-018..FR-023, v1's half (P1 K6).
//!
//! v1 keeps an entity's past in one table, `og_data.og_history`, filled by a
//! row trigger on each type table (`og_capture_history`), and provenance in
//! `og_data.og_source`. These were the tail of `crate::agent`; the agent
//! interface itself (schema, errors, limits) is the same in every edition.

use crate::catalog::types;
use pgrx::prelude::*;
use pgrx::JsonB;
use serde_json::{json, Value};

// --------------------------------------------------------------------------
// Temporal — spec 008 FR-018..FR-023
// --------------------------------------------------------------------------

/// Turn on history capture for a type. Off by default: it costs writes.
#[pg_extern]
fn og_enable_history(graph: &str, type_name: &str) {
    let gid = types::graph_id(graph);
    let tid = types::type_id(gid, type_name);
    for sub in crate::catalog::labeling::og_subtypes(tid) {
        let Some(table) = types::storage_table(sub) else { continue };
        let trig = format!("og_hist_{sub}");
        Spi::run(&format!(
            "CREATE OR REPLACE TRIGGER {trig}
               AFTER INSERT OR UPDATE OR DELETE ON {table}
               FOR EACH ROW EXECUTE FUNCTION og_capture_history()"
        ))
        .unwrap_or_else(|e| error!("failed to enable history on {table}: {e}"));
    }
    Spi::run_with_args(
        "INSERT INTO og_catalog.setting (key, value) VALUES ($1, 'on')
         ON CONFLICT (key) DO UPDATE SET value = 'on'",
        &[format!("history.{graph}.{type_name}").into()],
    )
    .ok();
}

/// Change history for one entity — spec 008 FR-022.
#[pg_extern(stable)]
fn og_history(id: i64) -> TableIterator<
    'static,
    (
        name!(recorded_at, pgrx::datum::TimestampWithTimeZone),
        name!(op, String),
        name!(payload, JsonB),
    ),
> {
    let rows: Vec<_> = Spi::connect(|client| {
        client
            .select(
                "SELECT recorded_at, op::text, payload FROM og_data.og_history
                  WHERE entity_id = $1 ORDER BY recorded_at DESC, hist_id DESC",
                None,
                &[id.into()],
            )
            .expect("history scan failed")
            .filter_map(|r| {
                Some((
                    r.get::<pgrx::datum::TimestampWithTimeZone>(1).ok()??,
                    r.get::<String>(2).ok()??,
                    r.get::<JsonB>(3).ok()?.unwrap_or(JsonB(json!({}))),
                ))
            })
            .collect()
    });
    TableIterator::new(rows)
}

/// State of one entity at a past instant — spec 008 FR-020, FR-021.
#[pg_extern(stable)]
fn og_as_of(id: i64, at: pgrx::datum::TimestampWithTimeZone) -> JsonB {
    let tracked = crate::spiu::one::<bool>(
        "SELECT true FROM og_data.og_history WHERE entity_id = $1 LIMIT 1",
        &[id.into()],
    )
    .ok()
    .flatten()
    .unwrap_or(false);
    if !tracked {
        error!(
            "no history is retained for entity {id}. enable it with \
             og_enable_history(graph, type) — returning the current value instead would be a lie"
        );
    }
    // Several writes in one transaction share `recorded_at`, so `hist_id`
    // breaks the tie; and an entity whose last change before `at` was its
    // deletion did not exist then — its final payload is the pre-delete row.
    crate::spiu::one::<JsonB>(
        "SELECT CASE WHEN op = 'd' THEN NULL ELSE payload END FROM og_data.og_history
          WHERE entity_id = $1 AND recorded_at <= $2
          ORDER BY recorded_at DESC, hist_id DESC LIMIT 1",
        &[id.into(), at.into()],
    )
    .ok()
    .flatten()
    .unwrap_or(JsonB(Value::Null))
}

/// Attach provenance metadata to an entity — spec 008 FR-015.
#[pg_extern]
fn og_set_source(
    entity_id: i64,
    source: &str,
    confidence: default!(Option<f32>, "NULL"),
    author: default!(Option<&str>, "NULL"),
) {
    Spi::run_with_args(
        "INSERT INTO og_data.og_source (entity_id, source, confidence, author)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (entity_id) DO UPDATE
            SET source = EXCLUDED.source, confidence = EXCLUDED.confidence,
                author = EXCLUDED.author, ingested_at = now()",
        &[entity_id.into(), source.into(), confidence.into(), author.into()],
    )
    .expect("provenance write failed");
}
