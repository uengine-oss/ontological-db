//! TypeQL write execution — spec 010 T019..T025, T045..T046.
//!
//! Writes run procedurally, once per binding row, inside the caller's
//! transaction. The one rule that shapes everything here is FR-016: an
//! attribute instance is identified by *type and value*, so `has genre
//! "fiction"` interns rather than creates. That is what makes two books share
//! one genre instance — and it is why the value column carries a UNIQUE index.
//!
//! The catalog checks (type kinds, `owns`, value types and constraints, roles)
//! and the error wording are here; every row is written and read through the
//! storage API ([`crate::layout::Layout`], `reports/ontological-v2/P1-K3-api.md`),
//! so this file names no storage table.

use super::ast::*;
use super::compile::{lit_str, Compiler};
use super::schema;
use crate::catalog::{labeling, types};
use crate::layout::{SqlExpr, TypeStore, WriteError};
use serde_json::{json, Value};
use std::collections::HashMap;

pub type WResult<T> = Result<T, String>;

/// One row of bindings flowing through the pipeline.
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub vars: HashMap<String, EnvVal>,
}

#[derive(Clone, Debug)]
pub enum EnvVal {
    /// Entity or relation instance.
    Instance(i64),
    /// Attribute instance: identity plus value.
    Attr(i64, Value),
    /// A plain value from `let`.
    Value(Value),
}

impl EnvVal {
    pub fn id(&self) -> Option<i64> {
        match self {
            EnvVal::Instance(i) | EnvVal::Attr(i, _) => Some(*i),
            EnvVal::Value(_) => None,
        }
    }
    pub fn value(&self) -> Value {
        match self {
            EnvVal::Instance(i) => json!(i),
            EnvVal::Attr(_, v) | EnvVal::Value(v) => v.clone(),
        }
    }
}

// --------------------------------------------------------------------------
// insert / put
// --------------------------------------------------------------------------

pub fn run_insert(gid: i32, pats: &[Pat], env: &mut Env) -> WResult<()> {
    // 1. Instances first, so role players and owners exist before they are used.
    for p in pats {
        if let Pat::Isa { var, ty, .. } | Pat::IsaAnon { var, ty } = p {
            if env.vars.contains_key(var) {
                continue;
            }
            let id = create_instance(gid, ty)?;
            env.vars.insert(var.clone(), EnvVal::Instance(id));
        }
    }

    // 2. Role players.
    for p in pats {
        if let Pat::Role { rel, role, player } = p {
            let rel_id = require_id(env, rel)?;
            let player_id = require_id(env, player)?;
            add_role_player(gid, rel_id, role.as_deref(), player_id)?;
        }
    }

    // 3. Ownership.
    for p in pats {
        if let Pat::Has { owner, attr, val } = p {
            let owner_id = require_id(env, owner)?;
            let (attr_name, value) = match (attr, val) {
                (Some(a), HasVal::Cmp(CmpOp::Eq, e)) => (a.clone(), eval(env, e)?),
                (Some(a), HasVal::Var(v)) => {
                    let Some(b) = env.vars.get(v) else {
                        return Err(format!("insert refers to unbound variable '${v}'"));
                    };
                    (a.clone(), b.value())
                }
                (Some(a), _) => {
                    return Err(format!("'has {a}' in an insert needs a value"));
                }
                (None, _) => return Err("'has' in an insert needs an attribute type".into()),
            };
            let (attr_id, stored) = intern_attribute(gid, owner_id, &attr_name, &value)?;
            link_has(gid, owner_id, attr_id)?;
            if let (Some(a), HasVal::Var(v)) = (attr, val) {
                let _ = a;
                env.vars.entry(v.clone()).or_insert(EnvVal::Attr(attr_id, stored));
            }
        }
    }

    for p in pats {
        if let Pat::Let { var, expr } = p {
            let v = eval(env, expr)?;
            env.vars.insert(var.clone(), EnvVal::Value(v));
        }
    }
    Ok(())
}

/// `put` — reuse an existing match, otherwise insert. FR-021.
pub fn run_put(gid: i32, pats: &[Pat], env: &mut Env) -> WResult<()> {
    if let Some(found) = find_one(gid, pats, env)? {
        for (k, v) in found {
            env.vars.insert(k, v);
        }
        return Ok(());
    }
    run_insert(gid, pats, env)
}

/// Run the patterns as a match against the current bindings, returning the
/// first row if there is one.
pub fn find_one(gid: i32, pats: &[Pat], env: &Env) -> WResult<Option<HashMap<String, EnvVal>>> {
    let mut c = Compiler::new(gid);
    for (k, v) in &env.vars {
        match v {
            EnvVal::Instance(i) | EnvVal::Attr(i, _) => {
                c.seed_column(k, i.to_string(), Some(i.to_string()))
            }
            EnvVal::Value(val) => c.seed_column(k, json_literal(val), None),
        }
    }
    c.compile_patterns(pats)?;
    let vars = c.bound_vars();
    if vars.is_empty() {
        return Ok(None);
    }
    let mut cols = Vec::new();
    for v in &vars {
        cols.push(format!("{} AS {}", c.var_id(v)?, quote_ident(&format!("i_{v}"))));
        cols.push(format!("{} AS {}", c.var_value(v)?, quote_ident(&format!("v_{v}"))));
    }
    let sql = format!(
        "SELECT {}{}{} LIMIT 1",
        cols.join(", "),
        c.from_sql(),
        c.where_sql()
    );

    let row: Option<Value> = crate::spiu::one::<pgrx::JsonB>(
        &format!("SELECT to_jsonb(t) FROM ({sql}) t"),
        &[],
    )
    .map_err(|e| format!("put lookup failed: {e}\n--- SQL ---\n{sql}"))?
    .map(|j| j.0);

    let Some(Value::Object(m)) = row else { return Ok(None) };
    let mut out = HashMap::new();
    for v in &vars {
        let id = m.get(&format!("i_{v}")).and_then(|x| x.as_i64());
        let val = m.get(&format!("v_{v}")).cloned().unwrap_or(Value::Null);
        out.insert(
            v.clone(),
            match (id, c.is_attr(v)) {
                (Some(i), true) => EnvVal::Attr(i, val),
                (Some(i), false) => EnvVal::Instance(i),
                (None, _) => EnvVal::Value(val),
            },
        );
    }
    Ok(Some(out))
}

// --------------------------------------------------------------------------
// Instance creation
// --------------------------------------------------------------------------

fn create_instance(gid: i32, ty: &str) -> WResult<i64> {
    let tid = types::try_type_id(gid, ty).ok_or_else(|| {
        let hint = types::nearest_type_names(gid, ty);
        if hint.is_empty() {
            format!("type '{ty}' is not defined in this graph")
        } else {
            format!("type '{ty}' is not defined. did you mean: {}", hint.join(", "))
        }
    })?;
    if types::type_kind(tid) == 'a' {
        return Err(format!(
            "'{ty}' is an attribute type; create it by owning a value ('has {ty} <value>'), \
             not with 'isa'"
        ));
    }
    let l = crate::layout::active();
    let store = l
        .type_store(tid)
        .ok_or_else(|| format!("'{ty}' is abstract and cannot be instantiated"))?;
    l.refuse_virtual_write(tid);
    l.create_instance(tid, &store).map_err(|e| format!("creating '{ty}' failed: {e}"))
}

// --------------------------------------------------------------------------
// Attributes
// --------------------------------------------------------------------------

/// Find or create the attribute instance for this type and value, enforcing
/// the ownership annotations on the way in.
fn intern_attribute(
    gid: i32,
    owner_id: i64,
    attr: &str,
    value: &Value,
) -> WResult<(i64, Value)> {
    let attr_tid = types::try_type_id(gid, attr)
        .ok_or_else(|| format!("attribute type '{attr}' is not defined"))?;
    if types::type_kind(attr_tid) != 'a' {
        return Err(format!("'{attr}' is not an attribute type"));
    }
    let l = crate::layout::active();
    let owner_tid = l.type_of_element(owner_id);
    let Some((_, anns)) = schema::owned_attribute(gid, owner_tid, attr) else {
        return Err(format!(
            "'{}' does not own '{attr}'. declare 'owns {attr}' on it first",
            crate::storage::type_name_of(owner_tid)
        ));
    };
    let store = l
        .type_store(attr_tid)
        .ok_or_else(|| format!("attribute type '{attr}' is abstract and has no instances"))?;
    let vt = schema::value_type_of(attr_tid)
        .ok_or_else(|| format!("attribute type '{attr}' has no value type"))?;
    let sql_ty = schema::value_type_sql(&vt)?;
    let lit = SqlExpr::new(typed_literal(value, sql_ty, attr)?);

    check_value_constraints(attr_tid, attr, value)?;
    check_ownership_constraints(gid, owner_id, owner_tid, attr_tid, attr, &anns, &lit, &store)?;

    if let Some(existing) = l
        .find_attribute(&store, &lit)
        .map_err(|e| format!("attribute lookup for '{attr}' failed: {e}"))?
    {
        return Ok((existing, value.clone()));
    }

    let id = l.create_attribute(attr_tid, &store, &lit).map_err(|e| match e {
        WriteError::Register(e) => format!("creating '{attr}' failed: {e}"),
        WriteError::Store(e) => format!("creating '{attr}' = {value} failed: {e}"),
    })?;
    Ok((id, value.clone()))
}

fn check_value_constraints(attr_tid: i32, attr: &str, value: &Value) -> WResult<()> {
    let params = crate::spiu::one::<pgrx::JsonB>(
        "SELECT params FROM og_catalog.og_constraint
          WHERE type_id = ANY($1) AND kind = 'values' LIMIT 1",
        &[labeling::og_supertypes(attr_tid).into()],
    )
    .ok()
    .flatten()
    .map(|j| j.0);
    let Some(p) = params else { return Ok(()) };

    if let Some(Value::Array(allowed)) = p.get("values") {
        if !allowed.is_empty() && !allowed.iter().any(|a| a == value) {
            let list: Vec<String> = allowed.iter().map(|a| a.to_string()).collect();
            return Err(format!(
                "{value} is not an allowed value for '{attr}'. allowed: {}",
                list.join(", ")
            ));
        }
    }
    if let Some(Value::Array(r)) = p.get("range") {
        let n = value.as_f64();
        if let (Some(v), Some(lo)) = (n, r.first().and_then(|x| x.as_f64())) {
            if v < lo {
                return Err(format!("{value} is below the declared range of '{attr}' ({lo}..)"));
            }
        }
        if let (Some(v), Some(hi)) = (n, r.get(1).and_then(|x| x.as_f64())) {
            if v > hi {
                return Err(format!("{value} is above the declared range of '{attr}' (..{hi})"));
            }
        }
    }
    Ok(())
}

fn check_ownership_constraints(
    gid: i32,
    owner_id: i64,
    owner_tid: i32,
    attr_tid: i32,
    attr: &str,
    anns: &Value,
    lit: &SqlExpr,
    store: &TypeStore,
) -> WResult<()> {
    let l = crate::layout::active();
    let has_tid = schema::ensure_has_type(gid);
    let key = anns.get("key").and_then(|v| v.as_bool()).unwrap_or(false);
    let unique = anns.get("unique").and_then(|v| v.as_bool()).unwrap_or(false);

    if key || unique {
        // The same value may not be owned twice across the owner's type line.
        let owners = labeling::og_subtypes(root_of(owner_tid));
        let clash = l
            .other_owner(has_tid, store, lit, owner_id, &owners)
            .map_err(|e| format!("uniqueness check for '{attr}' failed: {e}"))?;
        if let Some(other) = clash {
            return Err(format!(
                "'{attr}' is declared @{} but that value is already owned by instance {other}",
                if key { "key" } else { "unique" }
            ));
        }
    }

    if let Some(card) = anns.get("card").and_then(|v| v.as_array()) {
        if let Some(max) = card.get(1).and_then(|x| x.as_i64()) {
            let attrs = labeling::og_subtypes(attr_tid);
            let n = l
                .count_owned(has_tid, owner_id, &attrs)
                .map_err(|e| format!("cardinality check for '{attr}' failed: {e}"))?;
            if n >= max {
                return Err(format!(
                    "'{attr}' is declared @card(..{max}) and instance {owner_id} already owns {n}"
                ));
            }
        }
    }
    Ok(())
}

/// The topmost supertype, so a `@key` is unique across the whole hierarchy the
/// way TypeDB scopes it, not just within one concrete subtype.
fn root_of(tid: i32) -> i32 {
    let line = labeling::og_supertypes(tid);
    crate::spiu::one::<i32>(
        "SELECT t.type_id FROM og_catalog.type t
           JOIN og_catalog.type_label l ON l.type_id = t.type_id
          WHERE t.type_id = ANY($1)
          ORDER BY l.depth ASC LIMIT 1",
        &[line.into()],
    )
    .ok()
    .flatten()
    .unwrap_or(tid)
}

/// Ownership edge — in v1 the registry, the typed table and both directions of
/// the segments, in one transaction (spec 001 FR-012). Nothing when `owner`
/// already owns `attr`.
pub fn link_has(gid: i32, owner: i64, attr: i64) -> WResult<()> {
    let has_tid = schema::ensure_has_type(gid);
    crate::layout::active().link_has(has_tid, owner, attr).map_err(|e| match e {
        WriteError::Register(e) => format!("ownership registry insert failed: {e}"),
        WriteError::Store(e) => format!("ownership insert failed: {e}"),
    })
}

// --------------------------------------------------------------------------
// Role players
// --------------------------------------------------------------------------

fn add_role_player(gid: i32, rel_id: i64, role: Option<&str>, player: i64) -> WResult<()> {
    let l = crate::layout::active();
    let rel_tid = l.type_of_element(rel_id);
    let rel_name = crate::storage::type_name_of(rel_tid);
    let role_id = match role {
        Some(r) => schema::find_role(gid, rel_tid, r).ok_or_else(|| {
            format!("relation '{rel_name}' declares no role '{r}'")
        })?,
        None => {
            // Positional: pick the first declared role this player can fill and
            // that is not already taken on this instance.
            let candidates = super::compile::all_roles_of(rel_tid);
            let mut chosen = None;
            for c in candidates {
                if !l.role_filled(rel_id, c) {
                    chosen = Some(c);
                    break;
                }
            }
            chosen.ok_or_else(|| {
                format!(
                    "relation '{rel_name}' has no free role for a positional player; \
                     name the role explicitly"
                )
            })?
        }
    };
    l.add_role_player(rel_id, role_id, player)
        .map_err(|e| format!("role assignment failed: {e}"))?;
    Ok(())
}

// --------------------------------------------------------------------------
// delete / update
// --------------------------------------------------------------------------

pub fn run_delete(gid: i32, items: &[DeleteItem], env: &Env) -> WResult<()> {
    let l = crate::layout::active();
    for item in items {
        match &item.kind {
            DeleteKind::Instance(v) => {
                let id = require_id(env, v)?;
                delete_instance(gid, id)?;
            }
            DeleteKind::Has { owner, attr } => {
                let owner_id = require_id(env, owner)?;
                let attr_id = match env.vars.get(attr) {
                    Some(b) => b.id().ok_or_else(|| {
                        format!("'${attr}' is not an attribute instance")
                    })?,
                    None => return Err(format!("delete refers to unbound variable '${attr}'")),
                };
                unlink_has(gid, owner_id, attr_id);
            }
            DeleteKind::Links { rel, role, player } => {
                let rel_id = require_id(env, rel)?;
                let player_id = require_id(env, player)?;
                let role_ids = match role {
                    Some(r) => {
                        let tid = l.type_of_element(rel_id);
                        vec![schema::find_role(gid, tid, r)
                            .ok_or_else(|| format!("no role '{r}' on this relation"))?]
                    }
                    None => super::compile::all_roles_of(l.type_of_element(rel_id)),
                };
                l.remove_role_player(rel_id, player_id, &role_ids)
                    .map_err(|e| format!("role removal failed: {e}"))?;
            }
        }
    }
    Ok(())
}

/// `update $x has a v;` replaces every current value of that attribute type.
pub fn run_update(gid: i32, pats: &[Pat], env: &mut Env) -> WResult<()> {
    for p in pats {
        let Pat::Has { owner, attr: Some(attr), .. } = p else {
            return Err("'update' only supports 'has' replacements".into());
        };
        let owner_id = require_id(env, owner)?;
        let attr_tid = types::try_type_id(gid, attr)
            .ok_or_else(|| format!("attribute type '{attr}' is not defined"))?;
        let has_tid = schema::ensure_has_type(gid);
        let attrs = labeling::og_subtypes(attr_tid);
        let old = crate::layout::active().owned_attributes(has_tid, owner_id, &attrs);
        for a in old {
            unlink_has(gid, owner_id, a);
        }
    }
    run_insert(gid, pats, env)
}

fn unlink_has(gid: i32, owner: i64, attr: i64) {
    let has_tid = schema::ensure_has_type(gid);
    crate::layout::active().unlink_has(has_tid, owner, attr);
}

/// Deleting an instance also removes what referenced it, so the graph never
/// keeps a role assignment or ownership pointing at nothing (FR-038).
fn delete_instance(gid: i32, id: i64) -> WResult<()> {
    let l = crate::layout::active();
    let has_tid = schema::ensure_has_type(gid);

    // Ownerships in both directions.
    for (_, src, dst) in l.ownerships_of(has_tid, id) {
        unlink_has(gid, src, dst);
    }

    // Relations this instance plays a role in are removed with it: a relation
    // missing a player is not a smaller relation, it is a broken one.
    for r in l.relations_played(id) {
        delete_instance(gid, r)?;
    }

    // Then its own role players (as a relation), its row, its registry entry
    // and segments; a virtual type is refused on the way.
    l.remove_instance(id);
    Ok(())
}

// --------------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------------

fn require_id(env: &Env, var: &str) -> WResult<i64> {
    env.vars
        .get(var)
        .ok_or_else(|| format!("refers to unbound variable '${var}'"))?
        .id()
        .ok_or_else(|| format!("'${var}' is a value, not an instance"))
}

pub fn eval(env: &Env, e: &Expr) -> WResult<Value> {
    Ok(match e {
        Expr::Lit(l) => schema::lit_json(l),
        Expr::Var(v) => env
            .vars
            .get(v)
            .ok_or_else(|| format!("refers to unbound variable '${v}'"))?
            .value(),
        Expr::Neg(x) => {
            let v = eval(env, x)?;
            json!(-v.as_f64().unwrap_or(0.0))
        }
        Expr::Bin(l, op, r) => {
            let a = eval(env, l)?.as_f64().unwrap_or(0.0);
            let b = eval(env, r)?.as_f64().unwrap_or(0.0);
            json!(match *op {
                "+" => a + b,
                "-" => a - b,
                "*" => a * b,
                "/" => a / b,
                _ => a % b,
            })
        }
        Expr::Attr(..) | Expr::Call(..) => {
            return Err("expressions of this shape are not available in a write stage".into())
        }
    })
}

/// A SQL literal of the attribute's declared type. Values are escaped here and
/// nowhere else, so no attribute value ever reaches SQL unquoted.
fn typed_literal(v: &Value, sql_ty: &str, attr: &str) -> WResult<String> {
    let bad = |want: &str| format!("'{attr}' expects {want}, got {v}");
    Ok(match sql_ty {
        "text" => match v {
            Value::String(s) => lit_str(s),
            other => lit_str(&other.to_string()),
        },
        "int8" => match v.as_i64() {
            Some(i) => i.to_string(),
            None => return Err(bad("an integer")),
        },
        "float8" | "numeric" => match v.as_f64() {
            Some(f) => format!("{f}::{sql_ty}"),
            None => return Err(bad("a number")),
        },
        "bool" => match v.as_bool() {
            Some(b) => b.to_string(),
            None => return Err(bad("a boolean")),
        },
        "date" | "timestamp" | "timestamptz" | "interval" => match v.as_str() {
            Some(s) => format!("{}::{sql_ty}", lit_str(&s.replace('T', " "))),
            None => return Err(bad("a date or time")),
        },
        other => return Err(format!("unsupported storage type '{other}' for '{attr}'")),
    })
}

fn json_literal(v: &Value) -> String {
    match v {
        Value::String(s) => lit_str(s),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Null => "NULL".into(),
        other => lit_str(&other.to_string()),
    }
}

pub fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
