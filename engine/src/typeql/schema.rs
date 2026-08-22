//! `define` execution — spec 010 T010..T018.
//!
//! Maps TypeQL type declarations onto the spec 002 catalog. Three passes,
//! because a real `.tql` file forward-references freely: `entity book ... owns
//! isbn` appears long before `attribute isbn, value string`.
//!
//! 1. declare every type shell (name, kind, abstractness)
//! 2. link `sub` and re-label the hierarchy
//! 3. resolve value types, create storage, then roles / owns / plays
//!
//! Every pass is idempotent (FR-012): re-running a `define` is a no-op, which
//! is what makes loading a schema twice harmless.

use super::ast::*;
use crate::catalog::{labeling, types};
use pgrx::prelude::*;
use serde_json::{json, Value};

/// Name of the internal relation type carrying attribute ownership.
///
/// A `$` prefix cannot collide with a TypeQL label, so this type is invisible
/// to the language while still being an ordinary catalog citizen.
pub const HAS_TYPE: &str = "$has";

pub type SResult<T> = Result<T, String>;

/// TypeQL value type → PostgreSQL column type.
pub fn value_type_sql(v: &str) -> SResult<&'static str> {
    Ok(match v.to_ascii_lowercase().as_str() {
        "string" => "text",
        "integer" | "long" => "int8",
        "double" => "float8",
        "decimal" => "numeric",
        "boolean" => "bool",
        "date" => "date",
        "datetime" => "timestamp",
        "datetime-tz" | "datetime_tz" => "timestamptz",
        "duration" => "interval",
        other => {
            return Err(format!(
                "unknown value type '{other}'. supported: string, integer, double, decimal, \
                 boolean, date, datetime, datetime-tz, duration"
            ))
        }
    })
}

pub fn attr_table(tid: i32) -> String {
    format!("og_data.a_{tid}")
}

// --------------------------------------------------------------------------
// Entry point
// --------------------------------------------------------------------------

pub fn run_define(gid: i32, defs: &[Definition]) -> SResult<()> {
    ensure_has_type(gid);

    // Pass 1 — shells.
    for d in defs {
        if let Definition::Type { kind, name, anns, .. } = d {
            declare_shell(gid, kind, name, anns.abstract_)?;
        }
    }

    // Pass 2 — inheritance, then one relabel for the whole batch.
    let mut linked = false;
    for d in defs {
        if let Definition::Type { name, sub: Some(parent), kind, .. } = d {
            linked |= link_sub(gid, name, parent, kind)?;
        }
    }
    if linked {
        labeling::relabel_graph(gid);
    }

    // Pass 3 — storage, and plain role declarations.
    //
    // A real `.tql` file forward-references without apology: `entity book ...
    // plays contribution:work` is written long before `relation contribution`.
    // So roles must all exist before any `plays` is validated, and specialising
    // roles must wait for the roles they specialise.
    for d in defs {
        if let Definition::Type { kind, name, value_type, anns, relates, sub, .. } = d {
            let tid = types::type_id(gid, name);
            if *kind == Kind::Attribute {
                ensure_attr_storage(gid, tid, name, value_type.as_deref(), sub.as_deref())?;
            } else {
                ensure_instance_storage(tid, anns.abstract_);
            }
            for r in relates.iter().filter(|r| r.specialises.is_none()) {
                declare_role(gid, tid, name, r)?;
            }
        }
    }

    // Pass 4 — role specialisation.
    for d in defs {
        if let Definition::Type { name, relates, .. } = d {
            let tid = types::type_id(gid, name);
            for r in relates.iter().filter(|r| r.specialises.is_some()) {
                declare_role(gid, tid, name, r)?;
            }
        }
    }

    // Pass 5 — everything that needs the full picture.
    for d in defs {
        match d {
            Definition::Type { name, anns, owns, plays, .. } => {
                let tid = types::type_id(gid, name);
                for o in owns {
                    declare_owns(gid, tid, o)?;
                }
                for p in plays {
                    declare_plays(gid, tid, p)?;
                }
                if !anns.values.is_empty() || anns.range.is_some() {
                    store_value_constraints(tid, anns);
                }
            }
            Definition::Function(f) => store_function(gid, f),
        }
    }

    labeling::bump_schema_version(gid, "typeql define");
    Ok(())
}

// --------------------------------------------------------------------------
// Pass 1 & 2
// --------------------------------------------------------------------------

fn declare_shell(gid: i32, kind: &Kind, name: &str, is_abstract: bool) -> SResult<()> {
    let k = match kind {
        Kind::Entity => 'e',
        Kind::Relation => 'r',
        Kind::Attribute => 'a',
    };
    if let Some(tid) = types::try_type_id(gid, name) {
        let existing = types::type_kind(tid);
        if existing != k {
            return Err(format!(
                "'{name}' is already defined as {} and cannot be redefined as {}",
                kind_word(existing),
                kind_word(k)
            ));
        }
        return Ok(());
    }
    Spi::run_with_args(
        "INSERT INTO og_catalog.type (type_id, graph_id, name, kind, is_abstract)
         VALUES (nextval('og_catalog.type_id_seq')::int4, $1, $2, $3::text::\"char\", $4)",
        &[gid.into(), name.into(), k.to_string().into(), is_abstract.into()],
    )
    .map_err(|e| format!("declaring type '{name}' failed: {e}"))?;
    let tid = types::type_id(gid, name);
    Spi::run_with_args(
        "INSERT INTO og_data.og_id_alloc (type_id, next_id) VALUES ($1, 1)
         ON CONFLICT DO NOTHING",
        &[tid.into()],
    )
    .ok();
    Ok(())
}

fn kind_word(k: char) -> &'static str {
    match k {
        'e' => "an entity type",
        'r' => "a relation type",
        _ => "an attribute type",
    }
}

/// Returns true when a new parent link was added.
fn link_sub(gid: i32, name: &str, parent: &str, kind: &Kind) -> SResult<bool> {
    let tid = types::type_id(gid, name);
    let pid = types::try_type_id(gid, parent).ok_or_else(|| {
        let hint = types::nearest_type_names(gid, parent);
        if hint.is_empty() {
            format!("'{name}' cannot sub '{parent}': no such type")
        } else {
            format!("'{name}' cannot sub '{parent}': no such type. did you mean: {}", hint.join(", "))
        }
    })?;
    let pk = types::type_kind(pid);
    let k = match kind {
        Kind::Entity => 'e',
        Kind::Relation => 'r',
        Kind::Attribute => 'a',
    };
    if pk != k {
        return Err(format!(
            "'{name}' is {} and cannot sub '{parent}', which is {}",
            kind_word(k),
            kind_word(pk)
        ));
    }
    let existed = crate::spiu::one::<bool>(
        "SELECT true FROM og_catalog.type_parent WHERE type_id = $1 AND parent_id = $2",
        &[tid.into(), pid.into()],
    )
    .ok()
    .flatten()
    .unwrap_or(false);
    if existed {
        return Ok(false);
    }
    Spi::run_with_args(
        "INSERT INTO og_catalog.type_parent (type_id, parent_id) VALUES ($1, $2)",
        &[tid.into(), pid.into()],
    )
    .map_err(|e| format!("linking '{name}' sub '{parent}' failed: {e}"))?;
    Ok(true)
}

// --------------------------------------------------------------------------
// Pass 3 — storage
// --------------------------------------------------------------------------

/// Entity and (reified) relation instances live in a skinny node table; their
/// attributes are separate instances, so there are no property columns here.
fn ensure_instance_storage(tid: i32, is_abstract: bool) {
    if is_abstract || types::storage_table(tid).is_some() {
        return;
    }
    let table = types::node_table(tid);
    // `__ext` is not used by TypeQL — attributes are instances, not columns —
    // but the Cypher view builder projects it, and both languages read the
    // same tables (FR-040).
    Spi::run(&format!(
        "CREATE TABLE IF NOT EXISTS {table} (id int8 PRIMARY KEY, __ext jsonb)"
    ))
    .expect("instance table creation failed");
    crate::catalog::privileges::apply_to_table(&table);
    set_storage_table(tid, &table);
}

/// Attribute instances are deduplicated by value — that is what makes two books
/// share one `genre` instance (FR-016), so the UNIQUE index is load-bearing,
/// not merely defensive.
fn ensure_attr_storage(
    gid: i32,
    tid: i32,
    name: &str,
    value_type: Option<&str>,
    sub: Option<&str>,
) -> SResult<()> {
    let vt = match value_type {
        Some(v) => v.to_string(),
        None => match sub {
            Some(parent) => inherited_value_type(gid, parent).ok_or_else(|| {
                format!(
                    "attribute '{name}' has no value type and neither does its supertype '{parent}'"
                )
            })?,
            None => {
                // Abstract attribute types may legitimately omit it.
                if is_abstract(tid) {
                    return Ok(());
                }
                return Err(format!("attribute '{name}' needs a value type ('value string', ...)"));
            }
        },
    };
    record_value_type(tid, &vt);
    if is_abstract(tid) || types::storage_table(tid).is_some() {
        return Ok(());
    }
    let sql_ty = value_type_sql(&vt)?;
    let table = attr_table(tid);
    // UNIQUE on the value is load-bearing, not defensive: it is what makes two
    // owners of "fiction" share one instance (FR-016).
    Spi::run(&format!(
        "CREATE TABLE IF NOT EXISTS {table} \
         (id int8 PRIMARY KEY, val {sql_ty} NOT NULL UNIQUE, __ext jsonb)"
    ))
    .map_err(|e| format!("attribute storage for '{name}' failed: {e}"))?;
    crate::catalog::privileges::apply_to_table(&table);
    set_storage_table(tid, &table);
    // Declaring `val` as a property makes the attribute's value visible through
    // the Cypher surface without a second projection path.
    Spi::run_with_args(
        "INSERT INTO og_catalog.property
            (prop_id, type_id, name, data_type, column_name, required)
         VALUES (nextval('og_catalog.property_id_seq')::int4, $1, 'val', $2, 'val', true)
         ON CONFLICT (type_id, name) DO NOTHING",
        &[tid.into(), sql_ty.into()],
    )
    .ok();
    Ok(())
}

fn set_storage_table(tid: i32, table: &str) {
    Spi::run_with_args(
        "UPDATE og_catalog.type SET storage_table = $2 WHERE type_id = $1",
        &[tid.into(), table.into()],
    )
    .expect("storage_table update failed");
}

fn is_abstract(tid: i32) -> bool {
    crate::spiu::one::<bool>(
        "SELECT is_abstract FROM og_catalog.type WHERE type_id = $1",
        &[tid.into()],
    )
    .ok()
    .flatten()
    .unwrap_or(false)
}

/// The value type is kept as a constraint row so the catalog stays the single
/// source of truth and the schema dump can reproduce it.
fn record_value_type(tid: i32, vt: &str) {
    upsert_constraint(tid, "value", Some(vt), json!({}));
}

pub fn value_type_of(tid: i32) -> Option<String> {
    crate::spiu::one::<String>(
        "SELECT target FROM og_catalog.og_constraint WHERE type_id = $1 AND kind = 'value'",
        &[tid.into()],
    )
    .ok()
    .flatten()
}

fn inherited_value_type(gid: i32, parent: &str) -> Option<String> {
    let pid = types::try_type_id(gid, parent)?;
    for anc in labeling::og_supertypes(pid) {
        if let Some(v) = value_type_of(anc) {
            return Some(v);
        }
    }
    None
}

// --------------------------------------------------------------------------
// Pass 3 — roles, owns, plays
// --------------------------------------------------------------------------

fn declare_role(gid: i32, tid: i32, rel_name: &str, r: &Relates) -> SResult<()> {
    let existing = crate::spiu::one::<i32>(
        "SELECT role_id FROM og_catalog.role WHERE rel_type_id = $1 AND name = $2",
        &[tid.into(), r.role.clone().into()],
    )
    .ok()
    .flatten();

    let parent_role = match &r.specialises {
        Some(p) => Some(find_role(gid, tid, p).ok_or_else(|| {
            format!(
                "'{rel_name}' relates '{}' as '{p}', but no supertype of '{rel_name}' \
                 declares a role named '{p}'",
                r.role
            )
        })?),
        None => None,
    };

    if let Some(rid) = existing {
        if parent_role.is_some() {
            Spi::run_with_args(
                "UPDATE og_catalog.role SET parent_role_id = $2 WHERE role_id = $1",
                &[rid.into(), parent_role.into()],
            )
            .ok();
        }
        return Ok(());
    }

    let ordinal = crate::spiu::one::<i64>(
        "SELECT count(*) FROM og_catalog.role WHERE rel_type_id = $1",
        &[tid.into()],
    )
    .ok()
    .flatten()
    .unwrap_or(0) as i32;

    let (card_min, card_max) = match r.anns.card {
        Some((lo, hi)) => (lo as i32, hi.map(|h| h as i32)),
        None => (0, None),
    };

    Spi::run_with_args(
        "INSERT INTO og_catalog.role
            (role_id, rel_type_id, name, player_type_id, ordinal, card_min, card_max, parent_role_id)
         VALUES (nextval('og_catalog.role_id_seq')::int4, $1, $2, NULL, $3, $4, $5, $6)",
        &[
            tid.into(),
            r.role.clone().into(),
            ordinal.into(),
            card_min.into(),
            card_max.into(),
            parent_role.into(),
        ],
    )
    .map_err(|e| format!("declaring role '{}' on '{rel_name}' failed: {e}", r.role))?;
    Ok(())
}

/// Find a role by name on `tid` or any of its supertypes.
pub fn find_role(_gid: i32, tid: i32, role: &str) -> Option<i32> {
    crate::spiu::one::<i32>(
        "SELECT r.role_id FROM og_catalog.role r
          WHERE r.rel_type_id = ANY($1) AND r.name = $2
          ORDER BY r.rel_type_id DESC LIMIT 1",
        &[labeling::og_supertypes(tid).into(), role.into()],
    )
    .ok()
    .flatten()
}

fn declare_owns(gid: i32, tid: i32, o: &Owns) -> SResult<()> {
    if types::try_type_id(gid, &o.attribute).is_none() {
        let hint = types::nearest_type_names(gid, &o.attribute);
        return Err(if hint.is_empty() {
            format!("owns '{}': no such attribute type", o.attribute)
        } else {
            format!(
                "owns '{}': no such attribute type. did you mean: {}",
                o.attribute,
                hint.join(", ")
            )
        });
    }
    upsert_constraint(tid, "owns", Some(&o.attribute), annotations_json(&o.anns));
    Ok(())
}

fn declare_plays(gid: i32, tid: i32, p: &Plays) -> SResult<()> {
    let rid = types::try_type_id(gid, &p.relation)
        .ok_or_else(|| format!("plays '{}:{}': no such relation type", p.relation, p.role))?;
    if find_role(gid, rid, &p.role).is_none() {
        return Err(format!(
            "plays '{}:{}': relation '{}' declares no role '{}'",
            p.relation, p.role, p.relation, p.role
        ));
    }
    upsert_constraint(tid, "plays", Some(&format!("{}:{}", p.relation, p.role)), json!({}));
    Ok(())
}

fn store_value_constraints(tid: i32, anns: &Annotations) {
    upsert_constraint(tid, "values", None, annotations_json(anns));
}

pub fn annotations_json(a: &Annotations) -> Value {
    let mut m = serde_json::Map::new();
    if a.key {
        m.insert("key".into(), json!(true));
    }
    if a.unique {
        m.insert("unique".into(), json!(true));
    }
    if let Some((lo, hi)) = a.card {
        m.insert("card".into(), json!([lo, hi]));
    }
    if !a.values.is_empty() {
        m.insert("values".into(), Value::Array(a.values.iter().map(lit_json).collect()));
    }
    if let Some((lo, hi)) = &a.range {
        m.insert(
            "range".into(),
            json!([lo.as_ref().map(lit_json), hi.as_ref().map(lit_json)]),
        );
    }
    Value::Object(m)
}

pub fn lit_json(l: &Lit) -> Value {
    match l {
        Lit::Str(s) => json!(s),
        Lit::Int(i) => json!(i),
        Lit::Float(f) => json!(f),
        Lit::Bool(b) => json!(b),
        Lit::DateTime(s) => json!(s),
    }
}

fn upsert_constraint(tid: i32, kind: &str, target: Option<&str>, params: Value) {
    let existing = crate::spiu::one::<i32>(
        "SELECT con_id FROM og_catalog.og_constraint
          WHERE type_id = $1 AND kind = $2 AND target IS NOT DISTINCT FROM $3",
        &[tid.into(), kind.into(), target.into()],
    )
    .ok()
    .flatten();
    match existing {
        Some(id) => {
            Spi::run_with_args(
                "UPDATE og_catalog.og_constraint SET params = $2 WHERE con_id = $1",
                &[id.into(), pgrx::JsonB(params).into()],
            )
            .ok();
        }
        None => {
            Spi::run_with_args(
                "INSERT INTO og_catalog.og_constraint (con_id, type_id, kind, target, params)
                 VALUES (nextval('og_catalog.constraint_id_seq')::int4, $1, $2, $3, $4)",
                &[tid.into(), kind.into(), target.into(), pgrx::JsonB(params).into()],
            )
            .expect("constraint insert failed");
        }
    }
}

fn store_function(gid: i32, f: &FunctionDef) {
    let sig = json!({
        "params": f.params.iter().map(|(n, t)| json!({"name": n, "type": t})).collect::<Vec<_>>(),
        "stream": f.returns_stream,
        "returns": f.return_types,
    });
    Spi::run_with_args(
        "INSERT INTO og_catalog.typeql_function (graph_id, name, signature, body)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (graph_id, name) DO UPDATE
            SET signature = EXCLUDED.signature, body = EXCLUDED.body",
        &[gid.into(), f.name.clone().into(), pgrx::JsonB(sig).into(), f.source.clone().into()],
    )
    .expect("function insert failed");
}

// --------------------------------------------------------------------------
// The internal ownership relation type
// --------------------------------------------------------------------------

/// Type id of the `$has` relation for this graph, creating it on first use.
pub fn ensure_has_type(gid: i32) -> i32 {
    if let Some(tid) = types::try_type_id(gid, HAS_TYPE) {
        return tid;
    }
    Spi::run_with_args(
        "INSERT INTO og_catalog.type (type_id, graph_id, name, kind, is_abstract)
         VALUES (nextval('og_catalog.type_id_seq')::int4, $1, $2, 'r'::\"char\", false)",
        &[gid.into(), HAS_TYPE.into()],
    )
    .expect("internal $has type creation failed");
    let tid = types::type_id(gid, HAS_TYPE);
    let table = types::edge_table(tid);
    Spi::run(&format!(
        "CREATE TABLE IF NOT EXISTS {table} (id int8 PRIMARY KEY, src int8 NOT NULL, dst int8 NOT NULL)"
    ))
    .expect("$has table creation failed");
    crate::catalog::privileges::apply_to_table(&table);
    Spi::run(&format!("CREATE INDEX IF NOT EXISTS e_{tid}_src ON {table} (src)")).ok();
    Spi::run(&format!("CREATE INDEX IF NOT EXISTS e_{tid}_dst ON {table} (dst)")).ok();
    set_storage_table(tid, &table);
    Spi::run_with_args(
        "INSERT INTO og_data.og_id_alloc (type_id, next_id) VALUES ($1, 1) ON CONFLICT DO NOTHING",
        &[tid.into()],
    )
    .ok();
    labeling::relabel_graph(gid);
    tid
}

/// Attribute types this type (or any supertype) declares ownership of.
pub fn owned_attribute(gid: i32, owner_tid: i32, attr: &str) -> Option<(i32, Value)> {
    let attr_tid = types::try_type_id(gid, attr)?;
    let owner_line = labeling::og_supertypes(owner_tid);
    // Ownership of a supertype attribute covers its subtypes: declaring
    // `owns isbn` permits owning an `isbn-13`.
    let attr_line = labeling::og_supertypes(attr_tid);
    let params = crate::spiu::one::<pgrx::JsonB>(
        "SELECT c.params FROM og_catalog.og_constraint c
           JOIN og_catalog.type t ON t.name = c.target AND t.graph_id = $3
          WHERE c.type_id = ANY($1) AND c.kind = 'owns' AND t.type_id = ANY($2)
          LIMIT 1",
        &[owner_line.into(), attr_line.into(), gid.into()],
    )
    .ok()
    .flatten()
    .map(|j| j.0)?;
    Some((attr_tid, params))
}
