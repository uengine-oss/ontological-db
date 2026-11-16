//! Type, property and role management — spec 002 FR-001..FR-008, FR-015..FR-026.
//!
//! Declaring a type generates real DDL: a per-type table whose declared
//! properties are **actual typed columns**.  That is what makes spec 001's
//! "no per-row JSON parsing" requirement true rather than aspirational, and it
//! hands the PostgreSQL planner column statistics for free.
//!
//! This module is the catalog's half: lookups, label resolution, graphs, roles
//! and rules. The functions that create, alter and drop a type's storage along
//! with its catalog rows (`og_create_type`, `og_add_property`, `og_create_index`,
//! `og_drop_type`, `og_drop_graph`) are v1's and live in
//! `crate::store_v1::type_tables` (P1 K6).

use crate::catalog::labeling;
use pgrx::prelude::*;

/// Property types we are willing to materialise as columns. Anything else is
/// rejected at declaration time rather than failing later at write time.
pub fn map_data_type(decl: &str) -> String {
    let d = decl.trim().to_ascii_lowercase();
    match d.as_str() {
        "string" | "text" | "str" => "text".into(),
        "int" | "integer" | "int4" => "int4".into(),
        "long" | "bigint" | "int8" => "int8".into(),
        "float" | "double" | "float8" => "float8".into(),
        "real" | "float4" => "float4".into(),
        "bool" | "boolean" => "bool".into(),
        "datetime" | "timestamptz" | "timestamp" => "timestamptz".into(),
        "date" => "date".into(),
        "uuid" => "uuid".into(),
        "numeric" | "decimal" => "numeric".into(),
        "json" | "jsonb" => "jsonb".into(),
        "text[]" | "string[]" => "text[]".into(),
        "int[]" | "bigint[]" => "int8[]".into(),
        other => {
            // vector(1536), vector(768), ... — spec 004 embeddings live in slots.
            if let Some(rest) = other.strip_prefix("vector(") {
                if let Some(dims) = rest.strip_suffix(')') {
                    if dims.chars().all(|c| c.is_ascii_digit()) && !dims.is_empty() {
                        return format!("vector({dims})");
                    }
                }
            }
            error!(
                "unsupported property type '{decl}'. supported: string, int, long, float, bool, \
                 datetime, date, uuid, numeric, jsonb, text[], int[], vector(N)"
            )
        }
    }
}

pub fn try_type_name(tid: i32) -> Option<String> {
    crate::spiu::one::<String>(
        "SELECT name FROM og_catalog.type WHERE type_id = $1",
        &[tid.into()],
    )
    .ok()
    .flatten()
}

// --------------------------------------------------------------------------
// Lookups
// --------------------------------------------------------------------------

pub fn graph_id(name: &str) -> i32 {
    crate::spiu::one::<i32>(
        "SELECT graph_id FROM og_catalog.graph WHERE name = $1",
        &[name.into()],
    )
    .expect("graph lookup failed")
    .unwrap_or_else(|| error!("graph '{name}' does not exist"))
}

pub fn try_type_id(gid: i32, name: &str) -> Option<i32> {
    crate::spiu::one::<i32>(
        "SELECT type_id FROM og_catalog.type WHERE graph_id = $1 AND name = $2",
        &[gid.into(), name.into()],
    )
    .expect("type lookup failed")
}

pub fn type_id(gid: i32, name: &str) -> i32 {
    try_type_id(gid, name).unwrap_or_else(|| {
        let hint = nearest_type_names(gid, name);
        if hint.is_empty() {
            error!("type '{name}' does not exist in this graph");
        } else {
            error!("type '{name}' does not exist. did you mean: {}", hint.join(", "));
        }
    })
}

/// Resolve a Cypher label *set* — `(:A:B:C)` — to the single type that stands
/// for it.
///
/// Neo4j lets a node carry several independent labels; here a node has one type
/// and inherits the names above it. The two line up exactly when the labels in
/// a pattern form a chain: `(:_Entity:Doc)` is the node whose type is `Doc`,
/// because `Doc` already *is* an `_Entity`. So the label set resolves to its
/// most specific member, and a set with no most-specific member — two unrelated
/// labels — can match nothing, which is reported rather than guessed at.
///
/// Returns `Err(unmatchable)` for a set no node can satisfy, and `Err(unknown)`
/// naming the first label that does not exist.
pub fn resolve_label_set(gid: i32, graph: &str, labels: &[String]) -> Result<LabelMatch, String> {
    if labels.is_empty() {
        return Ok(LabelMatch::Any);
    }
    let mut ids = Vec::with_capacity(labels.len());
    for name in labels {
        match try_type_id(gid, name) {
            Some(t) => ids.push(t),
            None => {
                // A label nothing has ever been written under is not an error —
                // in Cypher it simply matches nothing, and a caller that probes
                // for a label before creating it relies on that. The spelling
                // hint still goes out, as a notice, so a typo stays findable
                // (spec 008 FR-008) without turning a legal query into a failure.
                let near = nearest_type_names(gid, name);
                if !near.is_empty() {
                    pgrx::notice!(
                        "label '{name}' does not exist in graph '{graph}' — matching nothing. \
                         did you mean: {}",
                        near.join(", ")
                    );
                }
                return Ok(LabelMatch::Nothing);
            }
        }
    }
    // The most specific label is the one that is a subtype of every other.
    let most_specific = ids
        .iter()
        .copied()
        .find(|c| ids.iter().all(|o| c == o || labeling::og_is_subtype(*c, *o)));
    match most_specific {
        Some(t) => Ok(LabelMatch::Type(t)),
        // Unrelated labels: no node carries both, so the pattern matches
        // nothing. Cypher would say the same, quietly.
        None => Ok(LabelMatch::Nothing),
    }
}

/// What a label set resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelMatch {
    /// No label written — every node qualifies.
    Any,
    /// One concrete type, subtypes included.
    Type(i32),
    /// Nothing can satisfy this label set. Not an error: an empty result.
    Nothing,
}

/// Same resolution as `resolve_label_set`, but on the write path, where an
/// unknown label is created instead of refused — that is what Neo4j does, and
/// standing in for Neo4j is the point.
///
/// Labels are read as widening-to-narrowing left to right: writing
/// `(:_Entity:Doc)` when `Doc` is new declares `Doc` as a subtype of `_Entity`.
/// That is the only reading under which a label list carries a hierarchy, and it
/// is the shape every `(:Super:Sub)` writer already uses.
pub fn resolve_or_create_label_set(gid: i32, graph: &str, labels: &[String], kind: &str) -> i32 {
    if labels.is_empty() {
        error!("CREATE requires a label per new node");
    }
    let mut parents: Vec<String> = Vec::new();
    for name in labels {
        match try_type_id(gid, name) {
            None => {
                crate::store_v1::type_tables::create_type_inner(graph, name, kind, &parents, false);
            }
            // The type already exists. It may still be unplaced: `CREATE
            // CONSTRAINT FOR (n:Doc) …` declares `Doc` before any node carries
            // it, and a constraint says nothing about where `Doc` sits in the
            // hierarchy. Left as a root, the first `(:_Entity:Doc)` write would
            // then fail forever — the labels are not on one chain, and no
            // amount of retrying changes that.
            //
            // So adopt it now, under the same left-to-right reading that
            // governs a freshly created label. Only when it is safe: a root
            // with no instances, which is exactly the "declared but never
            // used" case a constraint leaves behind.
            Some(tid) => crate::store_v1::type_tables::adopt_if_unplaced(gid, tid, name, &parents),
        }
        parents.push(name.clone());
    }
    match resolve_label_set(gid, graph, labels) {
        Ok(LabelMatch::Type(t)) => t,
        Ok(LabelMatch::Any) => error!("CREATE requires a label per new node"),
        Ok(LabelMatch::Nothing) => error!(
            "no node can carry the labels (:{}) at once — they are not on one \
             inheritance chain, so there is no type to create",
            labels.join(":")
        ),
        Err(e) => error!("{e}"),
    }
}

/// Nearest type names by edit distance — feeds spec 008 FR-008 (correctable
/// errors). Computed in Rust so the extension does not depend on
/// `fuzzystrmatch` being installed.
pub fn nearest_type_names(gid: i32, name: &str) -> Vec<String> {
    let all: Vec<String> = Spi::connect(|client| {
        client
            .select(
                "SELECT name FROM og_catalog.type WHERE graph_id = $1",
                None,
                &[gid.into()],
            )
            .map(|rows| rows.filter_map(|r| r.get::<String>(1).ok().flatten()).collect())
            .unwrap_or_default()
    });
    let target = name.to_ascii_lowercase();
    let mut scored: Vec<(usize, String)> = all
        .into_iter()
        .map(|n| (edit_distance(&n.to_ascii_lowercase(), &target), n))
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    // Only suggest names that are plausibly what the caller meant.
    let cutoff = (target.chars().count() / 2).max(2);
    scored
        .into_iter()
        .filter(|(d, _)| *d <= cutoff)
        .take(3)
        .map(|(_, n)| n)
        .collect()
}

/// Levenshtein distance, two-row variant.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

pub fn type_kind(tid: i32) -> char {
    crate::spiu::one::<String>(
        "SELECT kind::text FROM og_catalog.type WHERE type_id = $1",
        &[tid.into()],
    )
    .expect("kind lookup failed")
    .and_then(|s| s.chars().next())
    .unwrap_or_else(|| error!("type {tid} does not exist"))
}

// --------------------------------------------------------------------------
// Graphs, roles, rules
// --------------------------------------------------------------------------

#[pg_extern]
fn og_create_graph(name: &str) -> i32 {
    if let Some(existing) = crate::spiu::one::<i32>(
        "SELECT graph_id FROM og_catalog.graph WHERE name = $1",
        &[name.into()],
    )
    .expect("graph lookup failed")
    {
        return existing;
    }
    let gid = crate::spiu::one_mut::<i32>(
        "INSERT INTO og_catalog.graph (graph_id, name)
         VALUES (nextval('og_catalog.graph_id_seq')::int4, $1) RETURNING graph_id",
        &[name.into()],
    )
    .expect("graph insert failed")
    .unwrap();
    labeling::bump_schema_version(gid, "create graph");
    gid
}

pub fn storage_table(tid: i32) -> Option<String> {
    crate::spiu::one::<String>(
        "SELECT storage_table FROM og_catalog.type WHERE type_id = $1",
        &[tid.into()],
    )
    .expect("storage_table lookup failed")
}

/// Declare a role on a relation type — spec 002 FR-004..FR-006.
#[pg_extern]
fn og_add_role(
    graph: &str,
    rel_type: &str,
    role: &str,
    player_type: Option<&str>,
    ordinal: i32,
    card_min: default!(i32, 0),
    card_max: default!(Option<i32>, "NULL"),
) {
    let gid = graph_id(graph);
    let rid = type_id(gid, rel_type);
    if type_kind(rid) != 'r' {
        error!("'{rel_type}' is not a relation type; roles only exist on relations");
    }
    let player = player_type.map(|p| type_id(gid, p));
    Spi::run_with_args(
        "INSERT INTO og_catalog.role
            (role_id, rel_type_id, name, player_type_id, ordinal, card_min, card_max)
         VALUES (nextval('og_catalog.role_id_seq')::int4, $1, $2, $3, $4, $5, $6)
         ON CONFLICT (rel_type_id, name) DO UPDATE
            SET player_type_id = EXCLUDED.player_type_id,
                ordinal = EXCLUDED.ordinal,
                card_min = EXCLUDED.card_min,
                card_max = EXCLUDED.card_max",
        &[rid.into(), role.into(), player.into(), ordinal.into(), card_min.into(), card_max.into()],
    )
    .expect("role insert failed");
    labeling::bump_schema_version(gid, &format!("add role {rel_type}.{role}"));
}

/// Declare a relation characteristic driving inference — spec 002 FR-027.
#[pg_extern]
fn og_add_rule(
    graph: &str,
    rel_type: &str,
    characteristic: &str,
    target_type: default!(Option<&str>, "NULL"),
) {
    let gid = graph_id(graph);
    let rid = type_id(gid, rel_type);
    let c = characteristic.to_ascii_lowercase();
    if !["transitive", "symmetric", "reflexive", "inverse"].contains(&c.as_str()) {
        error!("unknown characteristic '{characteristic}' (transitive|symmetric|reflexive|inverse)");
    }
    if c == "inverse" && target_type.is_none() {
        error!("characteristic 'inverse' requires a target relation type");
    }
    let target = target_type.map(|t| type_id(gid, t));
    Spi::run_with_args(
        "INSERT INTO og_catalog.rule (rule_id, rel_type_id, characteristic, target_type_id)
         VALUES (nextval('og_catalog.rule_id_seq')::int4, $1, $2, $3)
         ON CONFLICT DO NOTHING",
        &[rid.into(), c.into(), target.into()],
    )
    .expect("rule insert failed");
    labeling::bump_schema_version(gid, &format!("add rule on {rel_type}"));
}

