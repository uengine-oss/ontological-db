//! Bitemporal types — spec 013.
//!
//! Three axes, kept apart because they answer different questions:
//!
//! | axis | question | where it lives |
//! |---|---|---|
//! | valid time  | when was this true in the world? | `__valid tstzrange` on every row |
//! | system time | since when have we believed it?  | `__sys_from` on the current row, `[__sys_from, __sys_to)` in history |
//! | schema      | what structure was it written under? | `og_catalog.schema_version.snapshot` |
//!
//! Storage is the SQL:2011 system-versioning shape: the type's ordinary table
//! holds what is believed now, and `og_data.h_<type_id>` beside it holds every
//! superseded belief plus the earlier valid-time slices that are still
//! believed. The capture is a trigger (`og__temporal` in `access.sql`), so raw
//! SQL is versioned exactly as Cypher is.
//!
//! Reads need nothing from the caller but the instant. The generated type views
//! (`cypher::views`) put the current and history rows of a temporal type behind
//! one relation filtered by `og_valid_at()` / `og_system_at()`, which read the
//! `og.valid_at` / `og.system_at` settings. The compiled SQL is the same for
//! every instant, so the plan cache is unaffected; when no instant is set the
//! history branch is a one-time filter that is never scanned.
//!
//! Query shapes that can only see the current state — adjacency-only hops over
//! temporal relations, variable-length traversal, unlabelled node scans,
//! TypeQL — raise `OG_TEMPORAL_UNSUPPORTED` when an instant is set instead of
//! quietly answering with today's data.

use crate::catalog::{labeling, types};
use crate::spiu;
use pgrx::prelude::*;

/// History table for a concrete type's storage table: `n_5` / `e_5` → `h_5`.
pub fn history_table(table: &str) -> Option<String> {
    let rel = table.strip_prefix("og_data.")?;
    let tid = rel.strip_prefix("n_").or_else(|| rel.strip_prefix("e_"))?;
    Some(format!("og_data.h_{tid}"))
}

/// Is this concrete type versioned (declared on it or on an ancestor)?
pub fn is_temporal(tid: i32) -> bool {
    spiu::one::<bool>(
        "SELECT true FROM og_catalog.temporal WHERE type_id = ANY($1) LIMIT 1",
        &[labeling::og_supertypes(tid).into()],
    )
    .ok()
    .flatten()
    .unwrap_or(false)
}

/// Concrete temporal types of one kind (`'n'` node tables, `'e'` edge tables)
/// in a graph.
pub fn temporal_tables(gid: i32, kind: char) -> Vec<i32> {
    let pattern = format!("og_data.{kind}\\_%");
    let candidates: Vec<i32> = Spi::connect(|client| {
        client
            .select(
                "SELECT type_id FROM og_catalog.type
                  WHERE graph_id = $1 AND storage_table LIKE $2 ORDER BY type_id",
                None,
                &[gid.into(), pattern.into()],
            )
            .map(|rows| rows.filter_map(|r| r.get::<i32>(1).ok().flatten()).collect())
            .unwrap_or_default()
    });
    candidates.into_iter().filter(|t| is_temporal(*t)).collect()
}

/// Make a graph that has temporal types refuse point-in-time reads through a
/// path that only sees the current state.
pub fn guard_current_only(gid: i32, what: &str) {
    if temporal_tables(gid, 'n').is_empty() && temporal_tables(gid, 'e').is_empty() {
        return;
    }
    Spi::run_with_args("SELECT og_require_current($1)", &[what.into()])
        .unwrap_or_else(|e| error!("{e}"));
}

/// Version a type and its subtypes — spec 013 FR-001.
///
/// Existing rows become valid for all time and believed from now: nothing was
/// recorded about them before this call, and claiming otherwise would let a
/// query about last year return today's values.
#[pg_extern]
fn og_enable_temporal(graph: &str, type_name: &str) {
    let gid = types::graph_id(graph);
    let tid = types::type_id(gid, type_name);
    if !matches!(types::type_kind(tid), 'e' | 'r') {
        error!("only entity and relation types can be temporal");
    }
    Spi::run_with_args(
        "INSERT INTO og_catalog.temporal (type_id) VALUES ($1) ON CONFLICT DO NOTHING",
        &[tid.into()],
    )
    .expect("temporal record failed");
    Spi::run_with_args(
        "INSERT INTO og_catalog.stamped (type_id) VALUES ($1) ON CONFLICT DO NOTHING",
        &[tid.into()],
    )
    .expect("stamping record failed");
    for sub in labeling::og_subtypes(tid) {
        enable_table(sub);
    }
    labeling::bump_schema_version(gid, &format!("enable temporal on {type_name}"));
}

/// Attach versioning to one concrete type's storage. Idempotent; also called
/// when a subtype of a temporal type is created.
pub fn enable_table(sub: i32) {
    let Some(table) = types::storage_table(sub) else { return };
    let Some(hist) = history_table(&table) else { return };
    crate::security::stamp_table(&table);
    Spi::run(&format!(
        "ALTER TABLE {table}
           ADD COLUMN IF NOT EXISTS __valid tstzrange NOT NULL
               DEFAULT tstzrange('-infinity', 'infinity', '[)'),
           ADD COLUMN IF NOT EXISTS __sys_from timestamptz NOT NULL DEFAULT now()"
    ))
    .unwrap_or_else(|e| error!("could not add validity columns to {table}: {e}"));

    let exists = spiu::one::<bool>("SELECT to_regclass($1) IS NOT NULL", &[hist.as_str().into()])
        .ok()
        .flatten()
        .unwrap_or(false);
    if !exists {
        Spi::run(&format!(
            "CREATE TABLE {hist} (LIKE {table});
             ALTER TABLE {hist} ADD COLUMN __sys_to timestamptz NOT NULL,
                                ADD COLUMN __op text NOT NULL;
             CREATE INDEX ON {hist} (id);
             CREATE INDEX ON {hist} USING gist (__valid) WHERE __sys_to = 'infinity'"
        ))
        .unwrap_or_else(|e| error!("could not create history table {hist}: {e}"));
        if table.starts_with("og_data.e_") {
            Spi::run(&format!("CREATE INDEX ON {hist} (src); CREATE INDEX ON {hist} (dst)"))
                .unwrap_or_else(|e| error!("could not index {hist}: {e}"));
        }
        // The capture trigger runs as its own owner. If that is not whoever
        // is creating this table, it still has to be able to write here.
        if let Some(owner) = spiu::one::<String>(
            "SELECT proowner::regrole::text FROM pg_proc WHERE proname = 'og__temporal' LIMIT 1",
            &[],
        )
        .ok()
        .flatten()
        {
            Spi::run(&format!("GRANT SELECT, INSERT ON {hist} TO {owner}")).ok();
        }
        crate::catalog::privileges::apply_to_table(&hist);
    } else {
        sync_history(sub);
    }
    Spi::run(&format!(
        "CREATE OR REPLACE TRIGGER og_b_temporal BEFORE INSERT OR UPDATE OR DELETE ON {table}
           FOR EACH ROW EXECUTE FUNCTION og__temporal()"
    ))
    .unwrap_or_else(|e| error!("could not attach the temporal trigger to {table}: {e}"));
}

/// Bring a history table's columns in line with its storage table after a
/// schema change: add what is missing, follow type changes. Columns the storage
/// table dropped are kept — they hold history.
pub fn sync_history(sub: i32) {
    let Some(table) = types::storage_table(sub) else { return };
    let Some(hist) = history_table(&table) else { return };
    let exists = spiu::one::<bool>("SELECT to_regclass($1) IS NOT NULL", &[hist.as_str().into()])
        .ok()
        .flatten()
        .unwrap_or(false);
    if !exists {
        return;
    }
    let cur = columns_with_types(&table);
    let old = columns_with_types(&hist);
    for (col, ty) in &cur {
        match old.iter().find(|(c, _)| c == col) {
            None => {
                Spi::run(&format!("ALTER TABLE {hist} ADD COLUMN {col} {ty}"))
                    .unwrap_or_else(|e| error!("could not add {col} to {hist}: {e}"));
            }
            Some((_, hty)) if hty != ty => {
                Spi::run(&format!(
                    "ALTER TABLE {hist} ALTER COLUMN {col} TYPE {ty} USING {col}::{ty}"
                ))
                .unwrap_or_else(|e| error!("could not retype {col} on {hist}: {e}"));
            }
            _ => {}
        }
    }
}

pub fn columns_with_types(table: &str) -> Vec<(String, String)> {
    Spi::connect(|client| {
        client
            .select(
                "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod)
                   FROM pg_attribute a
                  WHERE a.attrelid = $1::regclass AND a.attnum > 0 AND NOT a.attisdropped
                  ORDER BY a.attnum",
                None,
                &[table.into()],
            )
            .map(|rows| {
                rows.filter_map(|r| Some((r.get::<String>(1).ok()??, r.get::<String>(2).ok()??)))
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// SQL predicates selecting the rows of a temporal type visible at the
/// session's instant: (current table, open history rows, closed history rows).
///
/// History holds two kinds of row. A *closed* row (`__sys_to` finite) is a
/// belief since superseded; only a point-in-time query can see it, so it is
/// gated on `og_asof_active()` and never scanned otherwise. An *open* row
/// (`__sys_to = infinity`) is a valid-time slice still believed — the part of
/// a row that stayed true when it changed. That is part of the present: after a
/// change dated in the future, the value that holds until then lives only
/// there. A partial GiST index on `__valid` over open rows keeps that branch an
/// index probe.
pub fn visibility_predicates() -> (&'static str, &'static str, &'static str) {
    (
        "__valid @> og_valid_at() AND __sys_from <= og_system_at()",
        "__sys_to = 'infinity' AND __valid @> og_valid_at() AND __sys_from <= og_system_at()",
        "og_asof_active() AND __sys_to <> 'infinity' AND __valid @> og_valid_at() \
         AND __sys_from <= og_system_at() AND __sys_to > og_system_at()",
    )
}

/// One concrete temporal edge type as a `(id, src, dst)` relation at the
/// session's instant — what a hop reads instead of the adjacency segments,
/// which only know the present.
pub fn edge_relation(sub: i32) -> Option<String> {
    let table = types::storage_table(sub)?;
    let hist = history_table(&table)?;
    let (cur, open, closed) = visibility_predicates();
    Some(format!(
        "(SELECT id, src, dst FROM {table} WHERE {cur} \
          UNION ALL SELECT id, src, dst FROM {hist} WHERE {open} \
          UNION ALL SELECT id, src, dst FROM {hist} WHERE {closed})"
    ))
}

/// Temporal types in a graph — for clients deciding whether an instant applies.
#[pg_extern(stable)]
fn og_temporal_types(graph: &str) -> Vec<String> {
    let gid = types::graph_id(graph);
    Spi::connect(|client| {
        client
            .select(
                "SELECT t.name FROM og_catalog.temporal x
                   JOIN og_catalog.type t USING (type_id)
                  WHERE t.graph_id = $1 ORDER BY t.name",
                None,
                &[gid.into()],
            )
            .map(|rows| rows.filter_map(|r| r.get::<String>(1).ok().flatten()).collect())
            .unwrap_or_default()
    })
}
