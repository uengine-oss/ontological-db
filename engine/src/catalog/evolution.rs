//! Schema evolution — spec 012.
//!
//! A schema change on a graph that already holds data is not one statement. It
//! is four, and they are separate calls so that the slow one can run in batches
//! while the application keeps writing:
//!
//! | phase | what happens | who is blocked |
//! |---|---|---|
//! | **expand**   | the new structure is added beside the old one; new writes must satisfy both | a brief catalog lock |
//! | **migrate**  | existing rows are brought across, `batch` rows per call, resumable | nobody |
//! | **validate** | the data is checked against the new rule; violations are reported with ids | nobody |
//! | **contract** | the old structure is removed and the new one takes its name | a brief catalog lock |
//!
//! Each change is a row in `og_catalog.schema_change` that records its phase,
//! the schema versions it moved between, and a report — what it found, what it
//! did, where a batch left off. That record is what makes a schema change a
//! first-class object rather than a script someone ran once.
//!
//! Kinds: `add_property`, `drop_property`, `rename_property`,
//! `alter_property_type`, `set_required`, `add_unique`, `rename_type`,
//! `alter_role`, `add_parent`, `remove_parent`.
//!
//! Most of these never touch a row. A property name, a type name and a place in
//! the hierarchy are catalog entries that point at storage — the physical
//! column is `p_<name>` only by origin, and the table is `n_<type_id>` — so
//! renaming, re-parenting and dropping are catalog edits whose cost does not
//! grow with the data. Only a change of *representation* (a column's type)
//! rewrites rows, and that one runs in batches. Splitting and merging types is not here: an identifier encodes
//! its type, so moving a node between types means re-identifying it and
//! rewriting every adjacency entry that points at it (see the plan).

use crate::catalog::{labeling, types};
use crate::cypher::compile::sql_str;
use crate::spiu;
use pgrx::prelude::*;
use pgrx::JsonB;
use pgrx::PgSqlErrorCode;
use serde_json::{json, Map, Value};

const KINDS: &[&str] = &[
    "add_property",
    "drop_property",
    "rename_property",
    "alter_property_type",
    "set_required",
    "add_unique",
    "rename_type",
    "alter_role",
    "add_parent",
    "remove_parent",
];

struct Change {
    id: i32,
    gid: i32,
    graph: String,
    kind: String,
    spec: Value,
    phase: String,
    report: Value,
}

fn load(change_id: i32) -> Change {
    let row = Spi::connect(|client| {
        let mut t = client
            .select(
                "SELECT c.graph_id, g.name, c.kind, c.spec, c.phase, c.report
                   FROM og_catalog.schema_change c JOIN og_catalog.graph g USING (graph_id)
                  WHERE c.change_id = $1",
                Some(1),
                &[change_id.into()],
            )
            .ok()?;
        let r = t.next()?;
        Some((
            r.get::<i32>(1).ok()??,
            r.get::<String>(2).ok()??,
            r.get::<String>(3).ok()??,
            r.get::<JsonB>(4).ok()??.0,
            r.get::<String>(5).ok()??,
            r.get::<JsonB>(6).ok()??.0,
        ))
    });
    let Some((gid, graph, kind, spec, phase, report)) = row else {
        error!("schema change {change_id} does not exist");
    };
    Change { id: change_id, gid, graph, kind, spec, phase, report }
}

fn save(c: &Change, phase: &str) {
    Spi::run_with_args(
        "UPDATE og_catalog.schema_change
            SET phase = $2, report = $3, updated_at = now(),
                to_version = CASE WHEN $2 IN ('contracted','aborted')
                                  THEN (SELECT last_value FROM og_catalog.schema_version_seq)
                                  ELSE to_version END
          WHERE change_id = $1",
        &[c.id.into(), phase.into(), JsonB(c.report.clone()).into()],
    )
    .expect("schema change update failed");
}

fn require_phase(c: &Change, allowed: &[&str], doing: &str) {
    if !allowed.contains(&c.phase.as_str()) {
        ereport!(
            ERROR,
            PgSqlErrorCode::ERRCODE_OBJECT_NOT_IN_PREREQUISITE_STATE,
            format!(
                "OG_SCHEMA_PHASE: change {} is '{}'; {doing} needs it to be {}",
                c.id,
                c.phase,
                allowed.join(" or ")
            )
        );
    }
}

fn s<'a>(spec: &'a Value, key: &str) -> &'a str {
    spec.get(key).and_then(Value::as_str).unwrap_or_else(|| {
        error!("schema change needs \"{key}\"");
    })
}

fn report_set(c: &mut Change, key: &str, v: Value) {
    if !c.report.is_object() {
        c.report = json!({});
    }
    c.report.as_object_mut().unwrap().insert(key.to_string(), v);
}

/// The declaring type's view of a property: (column, sql type), checked to
/// exist.
fn property(tid: i32, prop: &str) -> (String, String) {
    let row = spiu::two::<String, String>(
        "SELECT column_name, data_type FROM og_catalog.property WHERE type_id = $1 AND name = $2",
        &[tid.into(), prop.into()],
    )
    .expect("property lookup failed");
    match row {
        (Some(c), Some(t)) => (c, t),
        _ => error!(
            "property '{prop}' is not declared on '{}'",
            types::try_type_name(tid).unwrap_or_default()
        ),
    }
}

fn tables(tid: i32) -> Vec<(i32, String)> {
    labeling::og_subtypes(tid)
        .into_iter()
        .filter_map(|sub| types::storage_table(sub).map(|t| (sub, t)))
        .collect()
}

fn hist(table: &str) -> Option<String> {
    let h = crate::temporal::history_table(table)?;
    spiu::one::<bool>("SELECT to_regclass($1) IS NOT NULL", &[h.as_str().into()])
        .ok()
        .flatten()
        .filter(|b| *b)
        .map(|_| h)
}

fn count(sql: &str) -> i64 {
    spiu::one::<i64>(sql, &[]).ok().flatten().unwrap_or(0)
}

/// `count` for a statement that writes (a data-modifying CTE).
fn count_mut(sql: &str) -> i64 {
    spiu::one_mut::<i64>(sql, &[])
        .unwrap_or_else(|e| error!("migration statement failed: {e}"))
        .unwrap_or(0)
}

fn sample_ids(sql: &str) -> Vec<i64> {
    Spi::connect(|client| {
        client
            .select(sql, Some(10), &[])
            .map(|rows| rows.filter_map(|r| r.get::<i64>(1).ok().flatten()).collect())
            .unwrap_or_default()
    })
}

/// Rebuild the readable per-type views after a column change.
fn refresh_alias_views(tid: i32) {
    for sub in labeling::og_subtypes(tid) {
        if let (Some(table), Some(name)) = (types::storage_table(sub), types::try_type_name(sub)) {
            types::ensure_alias_view(sub, &name, &table);
        }
    }
}

fn drop_alias_views(tid: i32) {
    crate::cypher::views::drop_all_views();
    for sub in labeling::og_subtypes(tid) {
        if let Some(name) = types::try_type_name(sub) {
            types::drop_alias_view(&name);
        }
    }
}

fn type_of(c: &Change) -> i32 {
    types::type_id(c.gid, s(&c.spec, "type"))
}

// --------------------------------------------------------------------------
// SQL surface
// --------------------------------------------------------------------------

/// Record a schema change and report what it would touch. Nothing is altered.
///
/// ```sql
/// SELECT og_schema_plan('kb', '{"kind":"alter_property_type",
///                               "type":"Person","property":"age","to":"int"}');
/// ```
#[pg_extern]
fn og_schema_plan(graph: &str, change: JsonB) -> i32 {
    let gid = types::graph_id(graph);
    let spec = change.0;
    let kind = spec.get("kind").and_then(Value::as_str).unwrap_or("").to_string();
    if !KINDS.contains(&kind.as_str()) {
        error!("unknown schema change kind '{kind}'. one of: {}", KINDS.join(", "));
    }
    let id = spiu::one_mut::<i32>(
        "INSERT INTO og_catalog.schema_change (graph_id, kind, spec, from_version)
         VALUES ($1, $2, $3, (SELECT last_value FROM og_catalog.schema_version_seq))
         RETURNING change_id",
        &[gid.into(), kind.as_str().into(), JsonB(spec).into()],
    )
    .expect("schema change insert failed")
    .unwrap();
    let mut c = load(id);
    let r = analyse(&c);
    c.report = r;
    save(&c, "planned");
    id
}

/// Add the new structure beside the old one.
#[pg_extern]
fn og_schema_expand(change_id: i32) -> JsonB {
    let mut c = load(change_id);
    require_phase(&c, &["planned"], "expand");
    expand(&mut c);
    save(&c, "expanded");
    labeling::bump_schema_version(c.gid, &format!("schema change {} expand ({})", c.id, c.kind));
    JsonB(c.report)
}

/// Bring up to `batch` existing rows per table across. Call until it returns
/// `{"done": true}`; each call is its own short piece of work.
#[pg_extern]
fn og_schema_migrate(change_id: i32, batch: default!(i32, 10000)) -> JsonB {
    let mut c = load(change_id);
    require_phase(&c, &["expanded", "migrating"], "migrate");
    let moved = migrate(&mut c, batch.max(1) as i64);
    let done = moved == 0;
    report_set(&mut c, "migrated_last_call", json!(moved));
    let total = c.report.get("migrated").and_then(Value::as_i64).unwrap_or(0) + moved;
    report_set(&mut c, "migrated", json!(total));
    report_set(&mut c, "migration_done", json!(done));
    save(&c, "migrating");
    JsonB(json!({ "change_id": c.id, "moved": moved, "done": done, "migrated": total }))
}

/// Check the data against the new rule. Moves to `validated` only when there
/// are no violations; otherwise reports how many and which.
#[pg_extern]
fn og_schema_validate(change_id: i32) -> JsonB {
    let mut c = load(change_id);
    require_phase(&c, &["expanded", "migrating", "validated"], "validate");
    let (violations, samples) = validate(&c);
    report_set(&mut c, "violations", json!(violations));
    report_set(&mut c, "violation_samples", json!(samples));
    let phase = if violations == 0 {
        finish_validation(&c);
        "validated".to_string()
    } else {
        c.phase.clone()
    };
    save(&c, &phase);
    JsonB(json!({ "change_id": c.id, "phase": phase, "violations": violations, "samples": samples }))
}

/// Remove the old structure. Only from `validated`.
#[pg_extern]
fn og_schema_contract(change_id: i32) -> JsonB {
    let mut c = load(change_id);
    require_phase(&c, &["validated"], "contract");
    contract(&mut c);
    labeling::bump_schema_version(c.gid, &format!("schema change {} contract ({})", c.id, c.kind));
    save(&c, "contracted");
    JsonB(c.report)
}

/// Undo an expand. Anything not yet contracted can be abandoned.
#[pg_extern]
fn og_schema_abort(change_id: i32) -> JsonB {
    let mut c = load(change_id);
    require_phase(&c, &["planned", "expanded", "migrating", "validated"], "abort");
    if c.phase != "planned" {
        abort(&mut c);
        labeling::bump_schema_version(c.gid, &format!("schema change {} aborted", c.id));
    }
    save(&c, "aborted");
    JsonB(json!({ "change_id": c.id, "phase": "aborted" }))
}

/// Run every phase in one transaction. For changes that are safe to make at
/// once — or on a graph small enough not to care. Fails, and changes nothing,
/// if validation finds violations.
#[pg_extern]
fn og_schema_apply(graph: &str, change: JsonB) -> JsonB {
    let id = og_schema_plan(graph, change);
    og_schema_expand(id);
    loop {
        let r = og_schema_migrate(id, 100_000);
        if r.0.get("done").and_then(Value::as_bool).unwrap_or(true) {
            break;
        }
    }
    let v = og_schema_validate(id);
    if v.0.get("phase").and_then(Value::as_str) != Some("validated") {
        ereport!(
            ERROR,
            PgSqlErrorCode::ERRCODE_CHECK_VIOLATION,
            format!(
                "OG_SCHEMA_VIOLATION: {} row(s) violate the change; nothing was applied. samples: {}",
                v.0.get("violations").cloned().unwrap_or(json!(0)),
                v.0.get("samples").cloned().unwrap_or(json!([]))
            ),
            "use og_schema_plan / expand / migrate / validate to fix the data before contracting"
        );
    }
    og_schema_contract(id)
}

/// Every schema change recorded for a graph, newest first.
#[pg_extern(stable)]
fn og_schema_changes(
    graph: &str,
) -> TableIterator<
    'static,
    (
        name!(change_id, i32),
        name!(kind, String),
        name!(phase, String),
        name!(spec, JsonB),
        name!(report, JsonB),
        name!(created_by, String),
        name!(created_at, pgrx::datum::TimestampWithTimeZone),
        name!(updated_at, pgrx::datum::TimestampWithTimeZone),
    ),
> {
    let gid = types::graph_id(graph);
    let rows: Vec<_> = Spi::connect(|client| {
        client
            .select(
                "SELECT change_id, kind, phase, spec, report, created_by, created_at, updated_at
                   FROM og_catalog.schema_change WHERE graph_id = $1 ORDER BY change_id DESC",
                None,
                &[gid.into()],
            )
            .expect("schema change scan failed")
            .filter_map(|r| {
                Some((
                    r.get::<i32>(1).ok()??,
                    r.get::<String>(2).ok()??,
                    r.get::<String>(3).ok()??,
                    r.get::<JsonB>(4).ok()??,
                    r.get::<JsonB>(5).ok()??,
                    r.get::<String>(6).ok()??,
                    r.get::<pgrx::datum::TimestampWithTimeZone>(7).ok()??,
                    r.get::<pgrx::datum::TimestampWithTimeZone>(8).ok()??,
                ))
            })
            .collect()
    });
    TableIterator::new(rows)
}

// --------------------------------------------------------------------------
// Per-kind behaviour
// --------------------------------------------------------------------------

fn analyse(c: &Change) -> Value {
    let spec = &c.spec;
    match c.kind.as_str() {
        "add_property" => {
            let tid = type_of(c);
            let prop = s(spec, "property");
            let dt = types::map_data_type(s(spec, "data_type"));
            let pending: i64 = tables(tid)
                .iter()
                .map(|(_, t)| count(&format!("SELECT count(*) FROM {t} WHERE __ext ? {}", sql_str(prop))))
                .sum();
            json!({ "type_id": tid, "data_type": dt, "values_in_ext": pending,
                    "lock": "brief ACCESS EXCLUSIVE per table (ADD COLUMN, no rewrite)" })
        }
        "drop_property" => {
            let tid = type_of(c);
            let prop = s(spec, "property");
            let (col, _) = property(tid, prop);
            declared_here(tid, prop);
            let n: i64 = tables(tid)
                .iter()
                .map(|(_, t)| count(&format!("SELECT count(*) FROM {t} WHERE {col} IS NOT NULL")))
                .sum();
            json!({ "type_id": tid, "column": col, "values_lost": n,
                    "note": "history tables keep the column" })
        }
        "rename_property" => {
            let tid = type_of(c);
            let prop = s(spec, "property");
            let new = s(spec, "new_name");
            if crate::security::is_reserved_key(new) {
                error!("'{new}' is a reserved name");
            }
            property(tid, prop);
            declared_here(tid, prop);
            if spiu::one::<bool>(
                "SELECT true FROM og_catalog.property WHERE type_id = ANY($1) AND name = $2 LIMIT 1",
                &[labeling::og_subtypes(tid).into(), new.into()],
            )
            .ok()
            .flatten()
            .is_some()
            {
                error!("'{new}' is already a property of '{}' or a subtype", s(spec, "type"));
            }
            json!({ "type_id": tid, "lock": "catalog only; no table is rewritten" })
        }
        "alter_property_type" => {
            let tid = type_of(c);
            let prop = s(spec, "property");
            let (col, from) = property(tid, prop);
            declared_here(tid, prop);
            let to = types::map_data_type(s(spec, "to"));
            if to == from {
                error!("'{prop}' is already {to}");
            }
            let mut bad = 0i64;
            let mut samples = Vec::new();
            for (_, t) in tables(tid) {
                let pred = format!("{col} IS NOT NULL AND NOT og__castable({col}::text, {})", sql_str(&to));
                bad += count(&format!("SELECT count(*) FROM {t} WHERE {pred}"));
                samples.extend(sample_ids(&format!("SELECT id FROM {t} WHERE {pred} ORDER BY id")));
            }
            json!({ "type_id": tid, "column": col, "from": from, "to_sql": to,
                    "shadow": format!("{col}__s{}", c.id),
                    "uncastable": bad, "uncastable_samples": samples,
                    "lock": "brief ACCESS EXCLUSIVE at expand (ADD COLUMN) and contract (DROP/RENAME)" })
        }
        "set_required" => {
            let tid = type_of(c);
            let prop = s(spec, "property");
            let (col, dt) = property(tid, prop);
            let (n, samples) = null_rows(tid, &col);
            json!({ "type_id": tid, "column": col, "data_type": dt, "missing": n,
                    "missing_samples": samples,
                    "fill": spec.get("default").cloned().unwrap_or(Value::Null),
                    "lock": "NOT VALID check at expand; VALIDATE takes SHARE UPDATE EXCLUSIVE (writes continue)" })
        }
        "add_unique" => {
            let tid = type_of(c);
            let prop = s(spec, "property");
            let (col, _) = property(tid, prop);
            let (n, samples) = duplicates(tid, &col);
            json!({ "type_id": tid, "column": col, "duplicate_values": n, "duplicate_samples": samples,
                    "note": "one unique index per concrete table, as og_add_property(is_key) builds" })
        }
        "rename_type" => {
            let tid = type_of(c);
            let new = s(spec, "new_name");
            if types::try_type_id(c.gid, new).is_some() {
                error!("type '{new}' already exists");
            }
            json!({ "type_id": tid, "lock": "catalog only; identifiers do not change" })
        }
        "add_parent" | "remove_parent" => {
            let tid = type_of(c);
            let pid = types::type_id(c.gid, s(spec, "parent"));
            if types::type_kind(tid) != types::type_kind(pid) {
                error!("'{}' and '{}' are different kinds of type", s(spec, "type"), s(spec, "parent"));
            }
            let linked = spiu::one::<bool>(
                "SELECT true FROM og_catalog.type_parent WHERE type_id = $1 AND parent_id = $2",
                &[tid.into(), pid.into()],
            )
            .ok()
            .flatten()
            .is_some();
            if c.kind == "add_parent" {
                if linked {
                    error!("'{}' already inherits from '{}'", s(spec, "type"), s(spec, "parent"));
                }
                if labeling::og_is_subtype(pid, tid) {
                    error!("'{}' is below '{}'; that would be a cycle", s(spec, "parent"), s(spec, "type"));
                }
                // Inherited properties that would collide with a different type.
                let clash = spiu::one::<String>(
                    "SELECT p.name FROM og_catalog.property p
                       JOIN og_catalog.property c ON c.name = p.name AND c.type_id = $2
                      WHERE p.type_id = ANY($1) AND p.data_type <> c.data_type LIMIT 1",
                    &[labeling::og_supertypes(pid).into(), tid.into()],
                )
                .ok()
                .flatten();
                if let Some(p) = clash {
                    error!("'{p}' is declared on both with different types");
                }
            } else if !linked {
                error!("'{}' does not inherit from '{}'", s(spec, "type"), s(spec, "parent"));
            }
            let gains: Vec<String> = Spi::connect(|client| {
                client
                    .select(
                        "SELECT DISTINCT name FROM og_catalog.property WHERE type_id = ANY($1)
                         EXCEPT SELECT name FROM og_catalog.property WHERE type_id = $2",
                        None,
                        &[labeling::og_supertypes(pid).into(), tid.into()],
                    )
                    .map(|rows| rows.filter_map(|r| r.get::<String>(1).ok().flatten()).collect())
                    .unwrap_or_default()
            });
            json!({ "type_id": tid, "parent_id": pid,
                    "gains_properties": if c.kind == "add_parent" { json!(gains) } else { json!([]) },
                    "rows_touched": 0,
                    "lock": "catalog only; inherited columns are added without a rewrite" })
        }
        "alter_role" => {
            let rid = types::type_id(c.gid, s(spec, "relation"));
            let role = s(spec, "role");
            let player = types::type_id(c.gid, s(spec, "player"));
            let (ordinal, role_id) = role_of(rid, role);
            let (n, samples) = role_violations(rid, ordinal, role_id, player);
            json!({ "relation_id": rid, "role_id": role_id, "ordinal": ordinal, "player_id": player,
                    "violations": n, "violation_samples": samples })
        }
        _ => unreachable!(),
    }
}

/// A property can be dropped, renamed or retyped only where it is declared —
/// on a subtype it is a copy of the ancestor's, and changing the copy would
/// leave the two disagreeing.
fn declared_here(tid: i32, prop: &str) {
    let above: Vec<i32> = labeling::og_supertypes(tid).into_iter().filter(|t| *t != tid).collect();
    if let Some(owner) = spiu::one::<String>(
        "SELECT t.name FROM og_catalog.property p JOIN og_catalog.type t USING (type_id)
          WHERE p.type_id = ANY($1) AND p.name = $2 LIMIT 1",
        &[above.into(), prop.into()],
    )
    .ok()
    .flatten()
    {
        error!("'{prop}' is inherited from '{owner}'; change it there");
    }
}

fn null_rows(tid: i32, col: &str) -> (i64, Vec<i64>) {
    let mut n = 0;
    let mut samples = Vec::new();
    for (_, t) in tables(tid) {
        n += count(&format!("SELECT count(*) FROM {t} WHERE {col} IS NULL"));
        samples.extend(sample_ids(&format!("SELECT id FROM {t} WHERE {col} IS NULL ORDER BY id")));
    }
    samples.truncate(10);
    (n, samples)
}

fn duplicates(tid: i32, col: &str) -> (i64, Vec<Value>) {
    let parts: Vec<String> = tables(tid)
        .iter()
        .map(|(_, t)| format!("SELECT {col}::text AS v FROM {t} WHERE {col} IS NOT NULL"))
        .collect();
    if parts.is_empty() {
        return (0, vec![]);
    }
    let dup = format!(
        "SELECT v FROM ({}) x GROUP BY v HAVING count(*) > 1",
        parts.join(" UNION ALL ")
    );
    let n = count(&format!("SELECT count(*) FROM ({dup}) d"));
    let samples: Vec<Value> = Spi::connect(|client| {
        client
            .select(&format!("{dup} ORDER BY v LIMIT 10"), None, &[])
            .map(|rows| rows.filter_map(|r| r.get::<String>(1).ok().flatten().map(Value::String)).collect())
            .unwrap_or_default()
    });
    (n, samples)
}

fn role_of(rid: i32, role: &str) -> (i32, i32) {
    match spiu::two::<i32, i32>(
        "SELECT ordinal, role_id FROM og_catalog.role WHERE rel_type_id = ANY($1) AND name = $2",
        &[labeling::og_supertypes(rid).into(), role.into()],
    )
    .expect("role lookup failed")
    {
        (Some(o), Some(r)) => (o, r),
        _ => error!("relation has no role named '{role}'"),
    }
}

fn role_violations(rid: i32, ordinal: i32, role_id: i32, player: i32) -> (i64, Vec<i64>) {
    let subs = labeling::og_subtypes(rid);
    let list = subs.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(",");
    let sql = match ordinal {
        0 | 1 => {
            let end = if ordinal == 0 { "src" } else { "dst" };
            format!(
                "SELECT e.id FROM og_data.og_edge e
                  WHERE e.type_id = ANY(ARRAY[{list}]::int4[])
                    AND NOT og_is_subtype(og_id_type(e.{end}), {player})"
            )
        }
        _ => format!(
            "SELECT rp.edge_id FROM og_data.og_role_player rp
              WHERE rp.role_id = {role_id} AND NOT og_is_subtype(og_id_type(rp.player_id), {player})"
        ),
    };
    let n = count(&format!("SELECT count(*) FROM ({sql}) v"));
    (n, sample_ids(&format!("{sql} ORDER BY 1")))
}

fn expand(c: &mut Change) {
    let spec = c.spec.clone();
    match c.kind.as_str() {
        "add_property" => {
            // Optional, so nothing existing can violate it: this is the whole
            // change, and og_add_property already brings __ext values across.
            let required = spec.get("required").and_then(Value::as_bool).unwrap_or(false);
            if required {
                error!("add the property optional, then use a set_required change");
            }
            Spi::run_with_args(
                "SELECT og_add_property($1, $2, $3, $4, false, false)",
                &[
                    c.graph.as_str().into(),
                    s(&spec, "type").into(),
                    s(&spec, "property").into(),
                    s(&spec, "data_type").into(),
                ],
            )
            .unwrap_or_else(|e| error!("{e}"));
        }
        "rename_property" => {
            // The new name is added as a second name for the same column:
            // readers and writers may use either until contract.
            let tid = type_of(c);
            let prop = s(&spec, "property");
            let new = s(&spec, "new_name");
            Spi::run_with_args(
                "INSERT INTO og_catalog.property
                     (prop_id, type_id, name, data_type, column_name, required, is_key,
                      card_min, card_max, domain_check, iri)
                 SELECT nextval('og_catalog.property_id_seq')::int4, type_id, $3, data_type,
                        column_name, required, is_key, card_min, card_max, domain_check, iri
                   FROM og_catalog.property WHERE type_id = ANY($1) AND name = $2",
                &[labeling::og_subtypes(tid).into(), prop.into(), new.into()],
            )
            .unwrap_or_else(|e| error!("could not add the new name: {e}"));
        }
        "alter_property_type" => {
            let tid = type_of(c);
            let shadow = c.report["shadow"].as_str().unwrap().to_string();
            let to = c.report["to_sql"].as_str().unwrap().to_string();
            let col = c.report["column"].as_str().unwrap().to_string();
            for (sub, t) in tables(tid) {
                Spi::run(&format!("ALTER TABLE {t} ADD COLUMN IF NOT EXISTS {shadow} {to}"))
                    .unwrap_or_else(|e| error!("could not add {shadow} to {t}: {e}"));
                // The dual write is a trigger, not engine code, so a raw SQL
                // write between expand and contract fills the new column too —
                // and a value the new type rejects is rejected now, not at
                // contract.
                Spi::run(&format!(
                    // `UPDATE OF col`: the backfill, which writes only the
                    // shadow, does not fire it at all.
                    "CREATE TRIGGER og_c_shadow_{id} BEFORE INSERT OR UPDATE OF {col} ON {t}
                       FOR EACH ROW EXECUTE FUNCTION og__shadow_write({}, {})",
                    sql_str(&col),
                    sql_str(&shadow),
                    id = c.id
                ))
                .unwrap_or_else(|e| error!("could not attach the dual write to {t}: {e}"));
                crate::temporal::sync_history(sub);
            }
            // The dual-write list is read from the change's spec.
            let mut sp = c.spec.as_object().cloned().unwrap_or_default();
            sp.insert("type_id".into(), json!(tid));
            sp.insert("column".into(), json!(col));
            sp.insert("shadow".into(), json!(shadow));
            sp.insert("to_sql".into(), json!(to));
            c.spec = Value::Object(sp);
            Spi::run_with_args(
                "UPDATE og_catalog.schema_change SET spec = $2 WHERE change_id = $1",
                &[c.id.into(), JsonB(c.spec.clone()).into()],
            )
            .expect("spec update failed");
        }
        "set_required" => {
            // NOT VALID: enforced for every write from now on, without
            // scanning what is already there.
            let col = c.report["column"].as_str().unwrap().to_string();
            for (_, t) in tables(type_of(c)) {
                Spi::run(&format!(
                    "ALTER TABLE {t} ADD CONSTRAINT og_req_{id} CHECK ({col} IS NOT NULL) NOT VALID",
                    id = c.id
                ))
                .unwrap_or_else(|e| error!("could not add the constraint to {t}: {e}"));
            }
        }
        // Checked, not structural: nothing to add before contract.
        "drop_property" | "add_unique" | "rename_type" | "alter_role" | "add_parent"
        | "remove_parent" => {}
        _ => unreachable!(),
    }
}

fn migrate(c: &mut Change, batch: i64) -> i64 {
    match c.kind.as_str() {
        "alter_property_type" => {
            let col = c.report["column"].as_str().unwrap().to_string();
            let shadow = c.report["shadow"].as_str().unwrap().to_string();
            let to = c.report["to_sql"].as_str().unwrap().to_string();
            let cast = format!(
                "CASE WHEN og__castable({col}::text, {ty}) THEN {col}::text::{to} END",
                ty = sql_str(&to)
            );
            let mut cursors: Map<String, Value> =
                c.report.get("cursor").and_then(Value::as_object).cloned().unwrap_or_default();
            let mut moved = 0i64;
            for (_, t) in tables(type_of(c)) {
                let from = cursors.get(&t).and_then(Value::as_i64).unwrap_or(i64::MIN);
                let (n, last) = batch_update(&t, &format!("{shadow} = {cast}"), &format!("{col} IS NOT NULL"), from, batch);
                moved += n;
                if let Some(l) = last {
                    cursors.insert(t.clone(), json!(l));
                }
                if let Some(h) = hist(&t) {
                    // History has no key to page on and is written only by the
                    // capture trigger; one statement per call.
                    moved += count_mut(&format!(
                        "WITH u AS (UPDATE {h} SET {shadow} = {cast}
                                     WHERE {col} IS NOT NULL AND {shadow} IS NULL
                                       AND og__castable({col}::text, {ty}) RETURNING 1)
                         SELECT count(*) FROM u",
                        ty = sql_str(&to)
                    ));
                }
            }
            report_set(c, "cursor", Value::Object(cursors));
            moved
        }
        "set_required" => {
            let Some(fill) = c.spec.get("default").filter(|v| !v.is_null()).cloned() else {
                return 0;
            };
            let col = c.report["column"].as_str().unwrap().to_string();
            let dt = c.report["data_type"].as_str().unwrap().to_string();
            let lit = match &fill {
                Value::String(v) => v.clone(),
                other => other.to_string(),
            };
            let mut moved = 0;
            for (_, t) in tables(type_of(c)) {
                moved += count_mut(&format!(
                    "WITH u AS (UPDATE {t} SET {col} = {}::{dt}
                                 WHERE id IN (SELECT id FROM {t} WHERE {col} IS NULL LIMIT {batch})
                                 RETURNING 1)
                     SELECT count(*) FROM u",
                    sql_str(&lit)
                ));
            }
            moved
        }
        _ => 0,
    }
}

/// Update one page of `table` in id order after `from`. Returns (rows, last id).
fn batch_update(table: &str, set: &str, filter: &str, from: i64, batch: i64) -> (i64, Option<i64>) {
    let row = spiu::two_mut::<i64, i64>(
        &format!(
            "WITH page AS (SELECT id FROM {table} WHERE id > {from} AND {filter} ORDER BY id LIMIT {batch}),
                  u AS (UPDATE {table} t SET {set} FROM page WHERE t.id = page.id RETURNING t.id)
             SELECT count(*), max(id) FROM u"
        ),
        &[],
    )
    .unwrap_or_else(|e| error!("migration batch on {table} failed: {e}"));
    (row.0.unwrap_or(0), row.1)
}

fn validate(c: &Change) -> (i64, Vec<Value>) {
    match c.kind.as_str() {
        "alter_property_type" => {
            let col = c.report["column"].as_str().unwrap();
            let shadow = c.report["shadow"].as_str().unwrap();
            let mut n = 0;
            let mut samples = Vec::new();
            for (_, t) in tables(type_of(c)) {
                let pred = format!("{col} IS NOT NULL AND {shadow} IS NULL");
                n += count(&format!("SELECT count(*) FROM {t} WHERE {pred}"));
                samples.extend(sample_ids(&format!("SELECT id FROM {t} WHERE {pred} ORDER BY id")));
                if let Some(h) = hist(&t) {
                    n += count(&format!("SELECT count(*) FROM {h} WHERE {pred}"));
                }
            }
            samples.truncate(10);
            (n, samples.into_iter().map(Value::from).collect())
        }
        "set_required" => {
            let col = c.report["column"].as_str().unwrap();
            let (n, s) = null_rows(type_of(c), col);
            (n, s.into_iter().map(Value::from).collect())
        }
        "add_unique" => {
            let col = c.report["column"].as_str().unwrap();
            duplicates(type_of(c), col)
        }
        "alter_role" => {
            let rid = c.report["relation_id"].as_i64().unwrap() as i32;
            let (n, s) = role_violations(
                rid,
                c.report["ordinal"].as_i64().unwrap() as i32,
                c.report["role_id"].as_i64().unwrap() as i32,
                c.report["player_id"].as_i64().unwrap() as i32,
            );
            (n, s.into_iter().map(Value::from).collect())
        }
        _ => (0, vec![]),
    }
}

/// Work that belongs to a successful validation.
fn finish_validation(c: &Change) {
    if c.kind == "set_required" {
        // Scans, but under SHARE UPDATE EXCLUSIVE: writes carry on.
        for (_, t) in tables(type_of(c)) {
            Spi::run(&format!("ALTER TABLE {t} VALIDATE CONSTRAINT og_req_{}", c.id))
                .unwrap_or_else(|e| error!("validation of {t} failed: {e}"));
        }
    }
}

fn contract(c: &mut Change) {
    let spec = c.spec.clone();
    match c.kind.as_str() {
        "add_property" => {}
        "drop_property" => {
            let tid = type_of(c);
            let col = c.report["column"].as_str().unwrap().to_string();
            let prop = s(&spec, "property");
            drop_alias_views(tid);
            for (_, t) in tables(tid) {
                Spi::run(&format!("ALTER TABLE {t} DROP COLUMN IF EXISTS {col}"))
                    .unwrap_or_else(|e| error!("could not drop {col} from {t}: {e}"));
            }
            Spi::run_with_args(
                "DELETE FROM og_catalog.property WHERE type_id = ANY($1) AND name = $2",
                &[labeling::og_subtypes(tid).into(), prop.into()],
            )
            .expect("property delete failed");
            refresh_alias_views(tid);
        }
        "rename_property" => {
            let tid = type_of(c);
            Spi::run_with_args(
                "DELETE FROM og_catalog.property WHERE type_id = ANY($1) AND name = $2",
                &[labeling::og_subtypes(tid).into(), s(&spec, "property").into()],
            )
            .expect("old name delete failed");
            // Write grants name properties; follow the rename.
            Spi::run_with_args(
                "UPDATE og_catalog.write_grant SET props = array_replace(props, $2, $3)
                  WHERE type_id = ANY($1)",
                &[
                    labeling::og_supertypes(tid).into(),
                    s(&spec, "property").into(),
                    s(&spec, "new_name").into(),
                ],
            )
            .ok();
            Spi::run_with_args(
                "UPDATE og_catalog.compat_index SET props = array_replace(props, $3, $4)
                  WHERE graph_id = $1 AND type_name = $2",
                &[c.gid.into(), s(&spec, "type").into(), s(&spec, "property").into(), s(&spec, "new_name").into()],
            )
            .ok();
        }
        "alter_property_type" => {
            let tid = type_of(c);
            let col = c.report["column"].as_str().unwrap().to_string();
            let shadow = c.report["shadow"].as_str().unwrap().to_string();
            let to = c.report["to_sql"].as_str().unwrap().to_string();
            drop_alias_views(tid);
            for (sub, t) in tables(tid) {
                // Indexes on the old column go with it; rebuild them on the new.
                let defs: Vec<String> = Spi::connect(|client| {
                    client
                        .select(
                            "SELECT pg_get_indexdef(i.indexrelid) FROM pg_index i
                               JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY(i.indkey)
                              WHERE i.indrelid = $1::regclass AND a.attname = $2 AND NOT i.indisprimary",
                            None,
                            &[t.as_str().into(), col.as_str().into()],
                        )
                        .map(|rows| rows.filter_map(|r| r.get::<String>(1).ok().flatten()).collect())
                        .unwrap_or_default()
                });
                Spi::run(&format!("DROP TRIGGER IF EXISTS og_c_shadow_{} ON {t}", c.id))
                    .unwrap_or_else(|e| error!("could not drop the dual write on {t}: {e}"));
                let mut rels = vec![t.clone()];
                rels.extend(hist(&t));
                for rel in &rels {
                    Spi::run(&format!(
                        "ALTER TABLE {rel} DROP COLUMN {col};
                         ALTER TABLE {rel} RENAME COLUMN {shadow} TO {col}"
                    ))
                    .unwrap_or_else(|e| error!("could not swap {col} on {rel}: {e}"));
                }
                for d in defs {
                    Spi::run(&d).unwrap_or_else(|e| error!("could not rebuild index ({d}): {e}"));
                }
                crate::security::replay_write_grants(sub);
            }
            Spi::run_with_args(
                "UPDATE og_catalog.property SET data_type = $3 WHERE type_id = ANY($1) AND column_name = $2",
                &[labeling::og_subtypes(tid).into(), col.as_str().into(), to.as_str().into()],
            )
            .expect("property update failed");
            refresh_alias_views(tid);
        }
        "set_required" => {
            let tid = type_of(c);
            let col = c.report["column"].as_str().unwrap().to_string();
            drop_alias_views(tid);
            for (_, t) in tables(tid) {
                // PostgreSQL 12+ proves NOT NULL from the validated CHECK and
                // skips the scan.
                Spi::run(&format!(
                    "ALTER TABLE {t} ALTER COLUMN {col} SET NOT NULL;
                     ALTER TABLE {t} DROP CONSTRAINT og_req_{}",
                    c.id
                ))
                .unwrap_or_else(|e| error!("could not tighten {t}: {e}"));
            }
            Spi::run_with_args(
                "UPDATE og_catalog.property SET required = true WHERE type_id = ANY($1) AND column_name = $2",
                &[labeling::og_subtypes(tid).into(), col.as_str().into()],
            )
            .expect("property update failed");
            refresh_alias_views(tid);
        }
        "add_unique" => {
            let tid = type_of(c);
            let col = c.report["column"].as_str().unwrap().to_string();
            for (sub, t) in tables(tid) {
                Spi::run(&format!("CREATE UNIQUE INDEX IF NOT EXISTS uq_{sub}_{col} ON {t} ({col})"))
                    .unwrap_or_else(|e| error!("could not build the unique index on {t}: {e}"));
            }
            Spi::run_with_args(
                "UPDATE og_catalog.property SET is_key = true WHERE type_id = ANY($1) AND column_name = $2",
                &[labeling::og_subtypes(tid).into(), col.as_str().into()],
            )
            .expect("property update failed");
        }
        "rename_type" => {
            let tid = type_of(c);
            let old = s(&spec, "type").to_string();
            let new = s(&spec, "new_name").to_string();
            Spi::run_with_args(
                "UPDATE og_catalog.type SET name = $2 WHERE type_id = $1",
                &[tid.into(), new.as_str().into()],
            )
            .expect("rename failed");
            types::drop_alias_view(&old);
            if let Some(table) = types::storage_table(tid) {
                types::ensure_alias_view(tid, &new, &table);
            }
            Spi::run_with_args(
                "UPDATE og_catalog.compat_index SET type_name = $3 WHERE graph_id = $1 AND type_name = $2",
                &[c.gid.into(), old.as_str().into(), new.as_str().into()],
            )
            .expect("index rename failed");
        }
        "add_parent" => {
            let tid = c.report["type_id"].as_i64().unwrap() as i32;
            let pid = c.report["parent_id"].as_i64().unwrap() as i32;
            Spi::run_with_args(
                "INSERT INTO og_catalog.type_parent (type_id, parent_id) VALUES ($1, $2)",
                &[tid.into(), pid.into()],
            )
            .expect("parent insert failed");
            // Every row of the type — and its subtypes — is now also a
            // `parent`. That is one interval relabel over the type DAG, not a
            // write per instance.
            labeling::relabel_graph(c.gid);
            for sub in labeling::og_subtypes(tid) {
                if let Some(table) = types::storage_table(sub) {
                    types::copy_inherited_properties(pid, sub, &table);
                    if crate::security::inherits_stamping(sub) {
                        crate::security::stamp_table(&table);
                    }
                    if crate::temporal::is_temporal(sub) {
                        crate::temporal::enable_table(sub);
                    }
                    crate::temporal::sync_history(sub);
                    crate::security::replay_write_grants(sub);
                }
            }
            refresh_alias_views(tid);
        }
        "remove_parent" => {
            // Properties inherited from the parent stay, as the type's own.
            let tid = c.report["type_id"].as_i64().unwrap() as i32;
            let pid = c.report["parent_id"].as_i64().unwrap() as i32;
            Spi::run_with_args(
                "DELETE FROM og_catalog.type_parent WHERE type_id = $1 AND parent_id = $2",
                &[tid.into(), pid.into()],
            )
            .expect("parent delete failed");
            labeling::relabel_graph(c.gid);
        }
        "alter_role" => {
            let role_id = c.report["role_id"].as_i64().unwrap() as i32;
            let player = c.report["player_id"].as_i64().unwrap() as i32;
            Spi::run_with_args(
                "UPDATE og_catalog.role
                    SET player_type_id = $2,
                        card_min = COALESCE($3, card_min),
                        card_max = CASE WHEN $5 THEN $4 ELSE card_max END
                  WHERE role_id = $1",
                &[
                    role_id.into(),
                    player.into(),
                    spec.get("card_min").and_then(Value::as_i64).map(|v| v as i32).into(),
                    spec.get("card_max").and_then(Value::as_i64).map(|v| v as i32).into(),
                    spec.get("card_max").is_some().into(),
                ],
            )
            .expect("role update failed");
        }
        _ => unreachable!(),
    }
}

fn abort(c: &mut Change) {
    match c.kind.as_str() {
        "rename_property" => {
            let tid = type_of(c);
            Spi::run_with_args(
                "DELETE FROM og_catalog.property WHERE type_id = ANY($1) AND name = $2",
                &[labeling::og_subtypes(tid).into(), s(&c.spec, "new_name").into()],
            )
            .expect("alias delete failed");
        }
        "alter_property_type" => {
            let tid = type_of(c);
            let shadow = c.report["shadow"].as_str().unwrap().to_string();
            drop_alias_views(tid);
            for (_, t) in tables(tid) {
                Spi::run(&format!("DROP TRIGGER IF EXISTS og_c_shadow_{} ON {t}", c.id))
                    .unwrap_or_else(|e| error!("could not drop the dual write on {t}: {e}"));
                let mut rels = vec![t.clone()];
                rels.extend(hist(&t));
                for rel in rels {
                    Spi::run(&format!("ALTER TABLE {rel} DROP COLUMN IF EXISTS {shadow}"))
                        .unwrap_or_else(|e| error!("could not drop {shadow} from {rel}: {e}"));
                }
            }
            refresh_alias_views(tid);
        }
        "set_required" => {
            for (_, t) in tables(type_of(c)) {
                Spi::run(&format!("ALTER TABLE {t} DROP CONSTRAINT IF EXISTS og_req_{}", c.id))
                    .unwrap_or_else(|e| error!("could not drop the constraint on {t}: {e}"));
            }
        }
        // add_property is complete at expand; aborting it leaves the property.
        _ => {}
    }
}
