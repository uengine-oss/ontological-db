//! Type, property and role management — spec 002 FR-001..FR-008, FR-015..FR-026.
//!
//! Declaring a type generates real DDL: a per-type table whose declared
//! properties are **actual typed columns**.  That is what makes spec 001's
//! "no per-row JSON parsing" requirement true rather than aspirational, and it
//! hands the PostgreSQL planner column statistics for free.

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

/// Physical column name for a declared property. Deterministic and injection-safe.
///
/// Letters outside ASCII are kept rather than folded to `_`: a graph with
/// Korean property names would otherwise map `이름` and `용량` to the same
/// column and silently merge two properties. PostgreSQL accepts those
/// characters in an unquoted identifier under UTF-8, so the name stays usable
/// as written. ASCII mapping is unchanged, so existing columns keep their names.
pub fn column_name(prop: &str) -> String {
    let mut s = String::with_capacity(prop.len() + 2);
    s.push_str("p_");
    for c in prop.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            s.push(c.to_ascii_lowercase());
        } else if c.is_alphanumeric() {
            s.extend(c.to_lowercase());
        } else {
            s.push('_');
        }
    }
    s
}

pub fn node_table(type_id: i32) -> String {
    format!("og_data.n_{type_id}")
}

pub fn edge_table(type_id: i32) -> String {
    format!("og_data.e_{type_id}")
}

/// Create (or move) the human-readable view for a type.
///
/// Named exactly as the type, so `og_data."MeetingRoom"` works in psql and in
/// any BI tool pointed at this database.
pub fn try_type_name(tid: i32) -> Option<String> {
    crate::spiu::one::<String>(
        "SELECT name FROM og_catalog.type WHERE type_id = $1",
        &[tid.into()],
    )
    .ok()
    .flatten()
}

pub fn ensure_alias_view(tid: i32, name: &str, table: &str) {
    let view = alias_view_name(name);
    // A type name is user input; quoting is what makes that safe here.
    let _ = Spi::run(&format!("DROP VIEW IF EXISTS {view}"));
    if let Err(e) = Spi::run(&format!(
        "CREATE VIEW {view}{} AS SELECT * FROM {table}",
        crate::spiu::VIEW_SECURITY
    )) {
        // Never fatal: the view is a convenience, and a name that collides with
        // something already in the schema must not stop a type from existing.
        pgrx::log!("could not create the alias view for type {tid} ({name}): {e}");
    } else {
        crate::catalog::privileges::apply_to_view(tid, &view);
    }
}

pub fn drop_alias_view(name: &str) {
    let _ = Spi::run(&format!("DROP VIEW IF EXISTS {}", alias_view_name(name)));
}

fn alias_view_name(name: &str) -> String {
    format!("og_data.\"{}\"", name.replace('"', "\"\""))
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
        if try_type_id(gid, name).is_none() {
            create_type_inner(graph, name, kind, &parents, false);
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
// Graph / type creation
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

#[pg_extern]
fn og_drop_graph(name: &str) {
    let gid = graph_id(name);
    Spi::connect(|client| {
        let rows = client
            .select(
                "SELECT storage_table FROM og_catalog.type
                  WHERE graph_id = $1 AND storage_table IS NOT NULL",
                None,
                &[gid.into()],
            )
            .expect("type lookup failed");
        let tables: Vec<String> = rows.filter_map(|r| r.get::<String>(1).ok().flatten()).collect();
        for t in tables {
            Spi::run(&format!("DROP TABLE IF EXISTS {t} CASCADE")).expect("drop table failed");
        }
    });
    Spi::run_with_args("DELETE FROM og_catalog.graph WHERE graph_id = $1", &[gid.into()])
        .expect("graph delete failed");
}

/// Declare a type.
///
/// * `kind` — `entity` | `relation` | `attribute`
/// * `parents` — supertypes; multiple entries make this multiple inheritance
///
/// Creates the backing storage table and re-labels the hierarchy.
#[pg_extern]
fn og_create_type(
    graph: &str,
    name: &str,
    kind: &str,
    parents: default!(Vec<Option<String>>, "'{}'"),
    is_abstract: default!(bool, false),
) -> i32 {
    let parent_names: Vec<String> = parents.into_iter().flatten().collect();
    create_type_inner(graph, name, kind, &parent_names, is_abstract)
}

/// The body of `og_create_type`, callable from Rust. The Cypher write path
/// needs it: Neo4j creates a label the first time it is written, and a graph
/// that refuses to do the same cannot stand in for one.
pub fn create_type_inner(
    graph: &str,
    name: &str,
    kind: &str,
    parents: &[String],
    is_abstract: bool,
) -> i32 {
    let gid = graph_id(graph);
    if try_type_id(gid, name).is_some() {
        error!("type '{name}' already exists in graph '{graph}'");
    }
    let k = match kind.to_ascii_lowercase().as_str() {
        "entity" | "e" | "node" => 'e',
        "relation" | "r" | "rel" | "edge" => 'r',
        "attribute" | "a" | "attr" => 'a',
        other => error!("unknown type kind '{other}' (entity | relation | attribute)"),
    };

    let parent_ids: Vec<i32> = parents
        .iter()
        .map(|p| {
            let pid = type_id(gid, p);
            let pk = type_kind(pid);
            if pk != k {
                error!("type '{name}' ({kind}) cannot inherit from '{p}' of kind '{pk}'");
            }
            pid
        })
        .collect();

    let tid = crate::spiu::one_mut::<i32>(
        "INSERT INTO og_catalog.type (type_id, graph_id, name, kind, is_abstract)
         VALUES (nextval('og_catalog.type_id_seq')::int4, $1, $2, $3::text::\"char\", $4)
         RETURNING type_id",
        &[gid.into(), name.into(), k.to_string().into(), is_abstract.into()],
    )
    .expect("type insert failed")
    .unwrap();

    for pid in &parent_ids {
        Spi::run_with_args(
            "INSERT INTO og_catalog.type_parent (type_id, parent_id) VALUES ($1, $2)",
            &[tid.into(), (*pid).into()],
        )
        .expect("parent insert failed");
    }

    // Storage table. Abstract types get none — they are never instantiated.
    if !is_abstract && k != 'a' {
        let table = if k == 'e' { node_table(tid) } else { edge_table(tid) };
        let base = if k == 'e' {
            format!("CREATE TABLE {table} (id int8 PRIMARY KEY, __ext jsonb)")
        } else {
            format!(
                "CREATE TABLE {table} (id int8 PRIMARY KEY, src int8 NOT NULL, \
                 dst int8 NOT NULL, __ext jsonb)"
            )
        };
        Spi::run(&base).expect("type table creation failed");
        crate::catalog::privileges::apply_to_table(tid, &table);
        // A view named after the type. The physical table is `n_<type_id>`
        // because a type can be renamed in constant time and its identifier
        // must not move; the cost is that `\dt` shows `n_45` and nothing about
        // MeetingRoom. This view puts the name back where a person looking at
        // the database can find it. It is a view, so it costs nothing to keep.
        ensure_alias_view(tid, name, &table);
        if k == 'r' {
            Spi::run(&format!("CREATE INDEX ON {table} (src)")).expect("index failed");
            Spi::run(&format!("CREATE INDEX ON {table} (dst)")).expect("index failed");
        }
        Spi::run_with_args(
            "UPDATE og_catalog.type SET storage_table = $2 WHERE type_id = $1",
            &[tid.into(), table.clone().into()],
        )
        .expect("storage_table update failed");

        // Inherited properties become columns on the child table too.
        for pid in &parent_ids {
            copy_inherited_properties(*pid, tid, &table);
        }
    }

    Spi::run_with_args(
        "INSERT INTO og_data.og_id_alloc (type_id, next_id) VALUES ($1, 1)
         ON CONFLICT DO NOTHING",
        &[tid.into()],
    )
    .expect("id alloc init failed");

    labeling::relabel_graph(gid);
    tid
}

fn copy_inherited_properties(parent: i32, child: i32, child_table: &str) {
    let props: Vec<(String, String, String, bool, bool)> = Spi::connect(|client| {
        client
            .select(
                "WITH RECURSIVE anc(type_id) AS (
                     SELECT $1
                     UNION
                     SELECT tp.parent_id FROM og_catalog.type_parent tp JOIN anc ON anc.type_id = tp.type_id
                 )
                 SELECT p.name, p.data_type, p.column_name, p.required, p.is_key
                   FROM og_catalog.property p JOIN anc ON anc.type_id = p.type_id",
                None,
                &[parent.into()],
            )
            .expect("inherited property lookup failed")
            .map(|r| {
                (
                    r.get::<String>(1).unwrap().unwrap(),
                    r.get::<String>(2).unwrap().unwrap(),
                    r.get::<String>(3).unwrap().unwrap(),
                    r.get::<bool>(4).unwrap().unwrap_or(false),
                    r.get::<bool>(5).unwrap().unwrap_or(false),
                )
            })
            .collect()
    });
    for (name, dtype, col, required, is_key) in props {
        Spi::run(&format!("ALTER TABLE {child_table} ADD COLUMN IF NOT EXISTS {col} {dtype}"))
            .expect("inherited column add failed");
        if required {
            Spi::run(&format!("ALTER TABLE {child_table} ALTER COLUMN {col} SET NOT NULL")).ok();
        }
        if is_key {
            Spi::run(&format!("CREATE UNIQUE INDEX ON {child_table} ({col})")).ok();
        }
        Spi::run_with_args(
            "INSERT INTO og_catalog.property
                (prop_id, type_id, name, data_type, column_name, required, is_key)
             VALUES (nextval('og_catalog.property_id_seq')::int4, $1, $2, $3, $4, $5, $6)
             ON CONFLICT (type_id, name) DO NOTHING",
            &[
                child.into(),
                name.into(),
                dtype.into(),
                col.into(),
                required.into(),
                is_key.into(),
            ],
        )
        .expect("inherited property record failed");
    }
}

/// Declare a property. Optional properties are added without rewriting existing
/// rows (PostgreSQL 11+ fast default) — spec 002 SC-004.
#[pg_extern]
fn og_add_property(
    graph: &str,
    type_name: &str,
    prop: &str,
    data_type: &str,
    required: default!(bool, false),
    is_key: default!(bool, false),
) {
    let gid = graph_id(graph);
    let tid = type_id(gid, type_name);
    let dtype = map_data_type(data_type);
    let col = column_name(prop);

    if required {
        let n = crate::spiu::one::<i64>(
            "SELECT count(*) FROM og_data.og_node WHERE type_id = ANY($1)",
            &[labeling::og_subtypes(tid).into()],
        )
        .expect("count failed")
        .unwrap_or(0);
        if n > 0 {
            error!(
                "cannot add required property '{prop}' to '{type_name}': {n} existing instance(s) \
                 would violate it. add it as optional, backfill, then tighten."
            );
        }
    }

    Spi::run_with_args(
        "INSERT INTO og_catalog.property
            (prop_id, type_id, name, data_type, column_name, required, is_key)
         VALUES (nextval('og_catalog.property_id_seq')::int4, $1, $2, $3, $4, $5, $6)",
        &[tid.into(), prop.into(), dtype.clone().into(), col.clone().into(), required.into(), is_key.into()],
    )
    .expect("property insert failed");

    // Apply to this type and every descendant (inheritance is by column copy).
    for sub in labeling::og_subtypes(tid) {
        if let Some(table) = storage_table(sub) {
            Spi::run(&format!("ALTER TABLE {table} ADD COLUMN IF NOT EXISTS {col} {dtype}"))
                .expect("column add failed");
            if required {
                Spi::run(&format!("ALTER TABLE {table} ALTER COLUMN {col} SET NOT NULL"))
                    .expect("not-null failed");
            }
            // Values written before the property was declared live in the
            // extension payload. Declaring the property has to bring them into
            // the column, or an index built on it would see nothing — which is
            // exactly what "write first, index later" produces, and that is the
            // ordinary order for an application coming from a schemaless graph.
            Spi::run(&format!(
                "UPDATE {table} SET {col} = (__ext ->> {}) ::{dtype}, __ext = __ext - {} \
                  WHERE __ext ? {}",
                crate::cypher::compile::sql_str(prop),
                crate::cypher::compile::sql_str(prop),
                crate::cypher::compile::sql_str(prop),
            ))
            .unwrap_or_else(|e| {
                error!("failed to move existing '{prop}' values into its column: {e}")
            });

            if is_key {
                Spi::run(&format!(
                    "CREATE UNIQUE INDEX IF NOT EXISTS uq_{sub}_{col} ON {table} ({col})"
                ))
                .expect("unique index failed");
            }
            if sub != tid {
                Spi::run_with_args(
                    "INSERT INTO og_catalog.property
                        (prop_id, type_id, name, data_type, column_name, required, is_key)
                     VALUES (nextval('og_catalog.property_id_seq')::int4, $1, $2, $3, $4, $5, $6)
                     ON CONFLICT (type_id, name) DO NOTHING",
                    &[sub.into(), prop.into(), dtype.clone().into(), col.clone().into(),
                      required.into(), is_key.into()],
                )
                .expect("descendant property record failed");
            }
        }
    }
    // A view freezes its column list when it is created, so the type's
    // human-readable view has to be rebuilt whenever the type gains a column —
    // otherwise `og_data."Room"` keeps showing the shape it had on day one.
    for sub in labeling::og_subtypes(tid) {
        if let (Some(table), Some(sub_name)) = (storage_table(sub), try_type_name(sub)) {
            ensure_alias_view(sub, &sub_name, &table);
        }
    }
    labeling::bump_schema_version(gid, &format!("add property {type_name}.{prop}"));
}

/// Create a secondary index on a declared property (spec 001 FR-016).
#[pg_extern]
fn og_create_index(graph: &str, type_name: &str, prop: &str) {
    let gid = graph_id(graph);
    let tid = type_id(gid, type_name);
    let col = column_name(prop);
    for sub in labeling::og_subtypes(tid) {
        if let Some(table) = storage_table(sub) {
            Spi::run(&format!("CREATE INDEX IF NOT EXISTS ix_{sub}_{col} ON {table} ({col})"))
                .expect("index creation failed");
        }
    }
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

/// Drop a type. Refuses when instances exist unless `cascade` — spec 002 FR-024.
#[pg_extern]
fn og_drop_type(graph: &str, name: &str, cascade: default!(bool, false)) {
    let gid = graph_id(graph);
    let tid = type_id(gid, name);
    let subs = labeling::og_subtypes(tid);
    let n = crate::spiu::one::<i64>(
        "SELECT (SELECT count(*) FROM og_data.og_node WHERE type_id = ANY($1))
              + (SELECT count(*) FROM og_data.og_edge WHERE type_id = ANY($1))",
        &[subs.clone().into()],
    )
    .expect("instance count failed")
    .unwrap_or(0);
    if n > 0 && !cascade {
        error!("type '{name}' has {n} instance(s) (including subtypes). pass cascade => true to remove them");
    }
    for sub in subs {
        if let Some(table) = storage_table(sub) {
            Spi::run(&format!("DROP TABLE IF EXISTS {table} CASCADE")).expect("drop failed");
        }
        Spi::run_with_args("DELETE FROM og_data.og_node WHERE type_id = $1", &[sub.into()]).ok();
        Spi::run_with_args("DELETE FROM og_data.og_edge WHERE type_id = $1", &[sub.into()]).ok();
        Spi::run_with_args("DELETE FROM og_data.og_adj WHERE etype = $1", &[sub.into()]).ok();
        Spi::run_with_args("DELETE FROM og_catalog.type WHERE type_id = $1", &[sub.into()])
            .expect("type delete failed");
    }
    labeling::relabel_graph(gid);
}
