//! TypeQL `match` → SQL — spec 010 T026..T036.
//!
//! One SQL statement per read query, so the PostgreSQL planner owns join order
//! and scan selection. The three joins that matter:
//!
//! * **isa** — `og_data.og_node` filtered by the subtype set that spec 002's
//!   interval labels hand back in one indexed range scan. Never a recursive walk.
//! * **has** — the `$has` adjacency segments, plus an attribute-type filter that
//!   is a *shift and mask on the neighbour id*: spec 001 puts the type in the
//!   identifier, so this costs no join at all.
//! * **role** — `og_data.og_role_player`, the table spec 002 created for exactly
//!   this, expanded over role specialisations so a supertype role reaches them.

use super::ast::*;
use super::schema;
use crate::catalog::{labeling, types};
use crate::id;
use pgrx::prelude::*;
use std::collections::{HashMap, HashSet};

pub type CResult<T> = Result<T, String>;

#[derive(Clone, Debug)]
pub enum Bind {
    /// Entity or (reified) relation instance: `<alias>.id`.
    Instance { alias: String },
    /// Attribute instance: `<alias>.id` and `<alias>.val`.
    Attr { alias: String, tids: Vec<i32> },
    /// A computed scalar from `let`.
    Value { expr: String },
    /// A type, from `$t sub x`.
    Type { alias: String },
    /// A variable already projected as a column by an enclosing pipeline stage.
    /// Referencing it is a correlated reference to that column, which is what
    /// lets `fetch` and sub-fetches survive being wrapped in sort/limit stages.
    Column { val: String, id: Option<String> },
}

pub struct Compiler {
    pub gid: i32,
    has_tid: i32,
    pub binds: HashMap<String, Bind>,
    /// Variables that come from an enclosing query; referencing them is a
    /// correlated reference, so they must not be re-materialised here.
    external: HashSet<String>,
    from: Vec<String>,
    wheres: Vec<String>,
    n: usize,
    /// Declared type per variable, from its `isa`. Role resolution needs it, so
    /// `isa` is compiled before anything else.
    isa_types: HashMap<String, i32>,
    /// Role-player join aliases per relation variable, so positional players of
    /// the same relation can be forced to be distinct assignments.
    role_aliases: Vec<(String, String)>,
}

impl Compiler {
    pub fn new(gid: i32) -> Self {
        Compiler {
            gid,
            has_tid: schema::ensure_has_type(gid),
            binds: HashMap::new(),
            external: HashSet::new(),
            from: Vec::new(),
            wheres: Vec::new(),
            n: 0,
            isa_types: HashMap::new(),
            role_aliases: Vec::new(),
        }
    }

    /// A compiler whose free variables resolve against an enclosing query.
    pub fn nested(parent: &Compiler) -> Self {
        let mut c = Compiler::new(parent.gid);
        c.n = parent.n + 1000;
        for (k, v) in &parent.binds {
            c.binds.insert(k.clone(), v.clone());
            c.external.insert(k.clone());
        }
        c.isa_types = parent.isa_types.clone();
        c
    }

    fn fresh(&mut self, hint: &str) -> String {
        self.n += 1;
        format!("{hint}{}", self.n)
    }

    /// Bind a variable to a column of an enclosing stage's output.
    pub fn seed_column(&mut self, var: &str, val: String, id: Option<String>) {
        self.binds.insert(var.to_string(), Bind::Column { val, id });
        self.external.insert(var.to_string());
    }

    /// Carry the declared type of a variable across a stage boundary, so role
    /// and attribute resolution still work after the columns were flattened.
    pub fn seed_type(&mut self, var: &str, tid: i32) {
        self.isa_types.insert(var.to_string(), tid);
    }

    pub fn declared_types(&self) -> HashMap<String, i32> {
        self.isa_types.clone()
    }

    pub fn is_bound(&self, v: &str) -> bool {
        self.binds.contains_key(v)
    }

    pub fn bound_vars(&self) -> Vec<String> {
        let mut v: Vec<String> =
            self.binds.keys().filter(|k| !self.external.contains(*k)).cloned().collect();
        v.sort();
        v
    }

    // ----------------------------------------------------------------------
    // Type resolution
    // ----------------------------------------------------------------------

    fn resolve(&self, name: &str) -> CResult<i32> {
        types::try_type_id(self.gid, name).ok_or_else(|| {
            let hint = types::nearest_type_names(self.gid, name);
            if hint.is_empty() {
                format!("type '{name}' is not defined in this graph")
            } else {
                format!("type '{name}' is not defined. did you mean: {}", hint.join(", "))
            }
        })
    }

    fn type_set(&self, name: &str, exact: bool) -> CResult<Vec<i32>> {
        let tid = self.resolve(name)?;
        Ok(if exact { vec![tid] } else { labeling::og_subtypes(tid) })
    }

    fn concrete_attr_tables(&self, tids: &[i32]) -> CResult<String> {
        let mut parts = Vec::new();
        for t in tids {
            if types::type_kind(*t) != 'a' {
                continue;
            }
            if let Some(tbl) = types::storage_table(*t) {
                parts.push(format!("SELECT id, val FROM {tbl}"));
            }
        }
        if parts.is_empty() {
            return Err("this attribute type has no instantiable subtype".into());
        }
        Ok(format!("({})", parts.join(" UNION ALL ")))
    }

    fn int_array(ids: &[i32]) -> String {
        format!(
            "ARRAY[{}]::int4[]",
            ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")
        )
    }

    /// `((id >> 36) & 262143)::int4` — the type of a node, straight from its
    /// identifier (spec 001 FR-004). No catalog lookup, no join.
    fn type_of_id(expr: &str) -> String {
        format!("((({expr}) >> {}) & {})::int4", id::TYPE_SHIFT, id::TYPE_MASK)
    }

    // ----------------------------------------------------------------------
    // Patterns
    // ----------------------------------------------------------------------

    pub fn compile_patterns(&mut self, pats: &[Pat]) -> CResult<()> {
        // Learn every variable's declared type first — role and attribute
        // resolution need it regardless of the order the author wrote things in.
        // This is a pure map update: emitting the `isa` joins first instead
        // would put all the type scans at the head of the FROM list, and a
        // `match` with ten independent variables would then have to build their
        // cross product before a single value filter could apply.
        self.scan_types(pats);
        for p in pats {
            self.compile_pat(p)?;
        }
        Ok(())
    }

    fn scan_types(&mut self, pats: &[Pat]) {
        for p in pats {
            match p {
                Pat::Isa { var, ty, .. } | Pat::IsaAnon { var, ty } => {
                    if let Some(tid) = types::try_type_id(self.gid, ty) {
                        self.isa_types.insert(var.clone(), tid);
                    }
                }
                Pat::Not(inner) => self.scan_types(inner),
                Pat::Or(branches) => {
                    for b in branches {
                        self.scan_types(b);
                    }
                }
                _ => {}
            }
        }
    }

    fn compile_pat(&mut self, p: &Pat) -> CResult<()> {
        match p {
            Pat::Isa { var, ty, exact } => self.pat_isa(var, ty, *exact),
            Pat::IsaAnon { var, ty } => self.pat_isa(var, ty, false),
            Pat::Has { owner, attr, val } => self.pat_has(owner, attr.as_deref(), val),
            Pat::Role { rel, role, player } => self.pat_role(rel, role.as_deref(), player),
            Pat::Cmp { lhs, op, rhs } => {
                let l = self.expr(lhs)?;
                let r = self.expr(rhs)?;
                let cond = Self::cmp_sql(&l, *op, &r);
                self.wheres.push(cond);
                Ok(())
            }
            Pat::Let { var, expr } => {
                let e = self.expr(expr)?;
                self.binds.insert(var.clone(), Bind::Value { expr: e });
                Ok(())
            }
            Pat::Not(inner) => {
                let sub = self.subquery(inner)?;
                self.wheres.push(format!("NOT EXISTS ({sub})"));
                Ok(())
            }
            Pat::Or(branches) => {
                let mut parts = Vec::new();
                for b in branches {
                    parts.push(format!("EXISTS ({})", self.subquery(b)?));
                }
                self.wheres.push(format!("({})", parts.join(" OR ")));
                Ok(())
            }
            Pat::Sub { var, ty, exact } => {
                let ids = self.type_set(ty, *exact)?;
                let a = self.fresh("ty");
                self.push_from(format!("og_catalog.type {a}"));
                self.wheres.push(format!("{a}.type_id = ANY({})", Self::int_array(&ids)));
                self.binds.insert(var.clone(), Bind::Type { alias: a });
                Ok(())
            }
            Pat::Label { var, name } => {
                let Some(Bind::Type { alias }) = self.binds.get(var).cloned() else {
                    return Err(format!(
                        "'${var} label {name}' needs ${var} to be a type variable (bind it with 'sub' first)"
                    ));
                };
                self.wheres.push(format!("{alias}.name = {}", lit_str(name)));
                Ok(())
            }
        }
    }

    fn push_from(&mut self, item: String) {
        if self.from.is_empty() {
            self.from.push(item);
        } else {
            self.from.push(format!("CROSS JOIN {item}"));
        }
    }

    fn pat_isa(&mut self, var: &str, ty: &str, exact: bool) -> CResult<()> {
        let tid = self.resolve(ty)?;
        let kind = types::type_kind(tid);
        let ids = self.type_set(ty, exact)?;
        self.isa_types.insert(var.to_string(), tid);

        if let Some(existing) = self.binds.get(var).cloned() {
            // Already bound — narrow it instead of joining a second time.
            let id_expr = match &existing {
                Bind::Instance { alias } | Bind::Attr { alias, .. } => format!("{alias}.id"),
                Bind::Value { expr } => expr.clone(),
                Bind::Column { val, id } => id.clone().unwrap_or_else(|| val.clone()),
                Bind::Type { .. } => {
                    return Err(format!("'${var}' is a type variable and cannot also be an instance"))
                }
            };
            self.wheres
                .push(format!("{} = ANY({})", Self::type_of_id(&id_expr), Self::int_array(&ids)));
            return Ok(());
        }

        if kind == 'a' {
            let src = self.concrete_attr_tables(&ids)?;
            let a = self.fresh("at");
            self.push_from(format!("{src} {a}"));
            self.binds.insert(var.to_string(), Bind::Attr { alias: a, tids: ids });
        } else {
            let a = self.fresh("n");
            self.push_from(format!("og_data.og_node {a}"));
            self.wheres.push(format!("{a}.type_id = ANY({})", Self::int_array(&ids)));
            self.binds.insert(var.to_string(), Bind::Instance { alias: a });
        }
        Ok(())
    }

    /// Bind a variable that has been referenced before its `isa` was seen.
    fn ensure_instance(&mut self, var: &str) -> CResult<String> {
        match self.binds.get(var) {
            Some(Bind::Instance { alias }) => Ok(format!("{alias}.id")),
            Some(Bind::Attr { alias, .. }) => Ok(format!("{alias}.id")),
            Some(Bind::Value { expr }) => Ok(expr.clone()),
            Some(Bind::Type { .. }) => {
                Err(format!("'${var}' is a type variable, not an instance"))
            }
            Some(Bind::Column { val, id }) => Ok(id.clone().unwrap_or_else(|| val.clone())),
            None => {
                let a = self.fresh("n");
                self.push_from(format!("og_data.og_node {a}"));
                self.binds.insert(var.to_string(), Bind::Instance { alias: a.clone() });
                Ok(format!("{a}.id"))
            }
        }
    }

    fn pat_has(&mut self, owner: &str, attr: Option<&str>, val: &HasVal) -> CResult<()> {
        let owner_id = self.ensure_instance(owner)?;
        let Some(attr_name) = attr else {
            return Err(
                "'has $a' without an attribute type is not supported: name the attribute type"
                    .into(),
            );
        };
        let attr_tids = self.type_set(attr_name, false)?;
        if types::type_kind(self.resolve(attr_name)?) != 'a' {
            return Err(format!("'{attr_name}' is not an attribute type"));
        }

        // Everything about one ownership — segment expansion, attribute-type
        // filter, and the value predicate — goes inside a single lateral. That
        // keeps the value filter next to the scan that produces the rows, so a
        // `match` over several independent variables never has to form their
        // cross product first.
        let h = self.fresh("h");
        let has_tid = self.has_tid;
        let src = self.concrete_attr_tables(&attr_tids)?;
        let mut inner = vec![
            format!("{h}a.src = {owner_id}"),
            format!("{h}a.dir = 'o'::\"char\""),
            format!("{h}a.etype = {has_tid}"),
            format!(
                "{} = ANY({})",
                Self::type_of_id(&format!("{h}u.nbr")),
                Self::int_array(&attr_tids)
            ),
        ];

        let mut bind_var: Option<String> = None;
        match val {
            HasVal::Any => {}
            HasVal::Var(v) => match self.binds.get(v).cloned() {
                Some(existing) => {
                    let e = match existing {
                        Bind::Attr { alias, .. } | Bind::Instance { alias } => format!("{alias}.id"),
                        Bind::Value { expr } => expr,
                        Bind::Column { val, id } => id.unwrap_or(val),
                        Bind::Type { .. } => return Err(format!("'${v}' is a type variable")),
                    };
                    inner.push(format!("{h}v.id = {e}"));
                }
                None => bind_var = Some(v.clone()),
            },
            HasVal::Cmp(op, e) => {
                let rhs = self.expr(e)?;
                inner.push(Self::cmp_sql(&format!("{h}v.val"), *op, &rhs));
            }
        }

        self.from.push(format!(
            "CROSS JOIN LATERAL (SELECT {h}v.id AS id, {h}v.val AS val \
             FROM og_data.og_adj {h}a \
             CROSS JOIN LATERAL unnest({h}a.nbr) AS {h}u(nbr) \
             JOIN {src} {h}v ON {h}v.id = {h}u.nbr \
             WHERE {}) {h}",
            inner.join(" AND ")
        ));
        if let Some(v) = bind_var {
            self.binds.insert(v, Bind::Attr { alias: h, tids: attr_tids });
        }
        Ok(())
    }

    fn pat_role(&mut self, rel: &str, role: Option<&str>, player: &str) -> CResult<()> {
        let rel_id = self.ensure_instance(rel)?;
        let player_id = self.ensure_instance(player)?;
        let rel_tid = self.static_type_of(rel);

        let role_ids = match role {
            Some(name) => {
                let Some(base) = rel_tid.and_then(|t| schema::find_role(self.gid, t, name)) else {
                    return Err(match rel_tid {
                        Some(t) => format!(
                            "relation '{}' declares no role '{name}'",
                            crate::storage::type_name_of(t)
                        ),
                        None => format!(
                            "cannot resolve role '{name}': the relation variable '${rel}' has no \
                             'isa' constraint, so its type is unknown"
                        ),
                    });
                };
                role_with_specialisations(base)
            }
            None => match rel_tid {
                Some(t) => all_roles_of(t),
                None => {
                    return Err(format!(
                        "a positional role player needs the relation type: add 'isa' to '${rel}'"
                    ))
                }
            },
        };
        if role_ids.is_empty() {
            return Err(format!("relation variable '${rel}' has no roles to match"));
        }

        let rp = self.fresh("rp");
        self.push_from(format!("og_data.og_role_player {rp}"));
        self.wheres.push(format!("{rp}.edge_id = {rel_id}"));
        self.wheres.push(format!("{rp}.player_id = {player_id}"));
        self.wheres.push(format!("{rp}.role_id = ANY({})", Self::int_array(&role_ids)));

        // Two positional players of the same relation must be distinct
        // assignments, otherwise `($a, $b) isa rating` would match one player
        // against itself.
        if role.is_none() {
            let prior: Vec<String> = self
                .role_aliases
                .iter()
                .filter(|(r, _)| r == rel)
                .map(|(_, a)| a.clone())
                .collect();
            for p in prior {
                self.wheres
                    .push(format!("NOT ({rp}.role_id = {p}.role_id AND {rp}.player_id = {p}.player_id)"));
            }
        }
        self.role_aliases.push((rel.to_string(), rp));
        Ok(())
    }

    /// The declared type of an instance variable, when an `isa` fixed it.
    fn static_type_of(&self, var: &str) -> Option<i32> {
        self.isa_types.get(var).copied()
    }

    // ----------------------------------------------------------------------
    // Expressions
    // ----------------------------------------------------------------------

    pub fn expr(&mut self, e: &Expr) -> CResult<String> {
        Ok(match e {
            Expr::Lit(l) => lit_sql(l),
            Expr::Var(v) => match self.binds.get(v) {
                Some(Bind::Attr { alias, .. }) => format!("{alias}.val"),
                Some(Bind::Instance { alias }) => format!("{alias}.id"),
                Some(Bind::Value { expr }) => format!("({expr})"),
                Some(Bind::Type { alias }) => format!("{alias}.name"),
                Some(Bind::Column { val, .. }) => val.clone(),
                None => return Err(format!("'${v}' is not bound by any pattern")),
            },
            Expr::Attr(v, a) => self.attr_scalar(v, a)?,
            Expr::Bin(l, op, r) => {
                let ls = self.expr(l)?;
                let rs = self.expr(r)?;
                format!("(({ls})::numeric {op} ({rs})::numeric)")
            }
            Expr::Neg(x) => {
                let s = self.expr(x)?;
                format!("(-({s})::numeric)")
            }
            Expr::Call(name, args) => {
                let mut a = Vec::new();
                for arg in args {
                    a.push(self.expr(arg)?);
                }
                match name.to_ascii_lowercase().as_str() {
                    "round" => format!("round(({})::numeric)", a.first().cloned().unwrap_or_default()),
                    "abs" => format!("abs(({})::numeric)", a.first().cloned().unwrap_or_default()),
                    "floor" => format!("floor(({})::numeric)", a.first().cloned().unwrap_or_default()),
                    "ceil" | "ceiling" => {
                        format!("ceil(({})::numeric)", a.first().cloned().unwrap_or_default())
                    }
                    "length" => format!("length(({})::text)", a.first().cloned().unwrap_or_default()),
                    other => {
                        return Err(format!(
                            "function '{other}' is not available in expressions. \
                             built-ins: round, abs, floor, ceil, length. user-defined \
                             functions are spec 010 phase 6"
                        ))
                    }
                }
            }
        })
    }

    /// `$book.title` — the single value of an owned attribute.
    pub fn attr_scalar(&mut self, var: &str, attr: &str) -> CResult<String> {
        let owner_id = self.ensure_instance(var)?;
        let tids = self.type_set(attr, false)?;
        let src = self.concrete_attr_tables(&tids)?;
        let h = self.fresh("p");
        let has_tid = self.has_tid;
        Ok(format!(
            "(SELECT {h}v.val FROM og_data.og_adj {h}a \
              CROSS JOIN LATERAL unnest({h}a.nbr) AS {h}u(nbr) \
              JOIN {src} {h}v ON {h}v.id = {h}u.nbr \
              WHERE {h}a.src = {owner_id} AND {h}a.dir = 'o'::\"char\" \
              AND {h}a.etype = {has_tid} \
              AND {} = ANY({}) LIMIT 1)",
            Self::type_of_id(&format!("{h}u.nbr")),
            Self::int_array(&tids)
        ))
    }

    /// `[$book.genre]` — every value of a multi-valued attribute.
    pub fn attr_list(&mut self, var: &str, attr: &str) -> CResult<String> {
        let owner_id = self.ensure_instance(var)?;
        let tids = self.type_set(attr, false)?;
        let src = self.concrete_attr_tables(&tids)?;
        let h = self.fresh("p");
        let has_tid = self.has_tid;
        Ok(format!(
            "COALESCE((SELECT jsonb_agg(to_jsonb({h}v.val)) FROM og_data.og_adj {h}a \
              CROSS JOIN LATERAL unnest({h}a.nbr) AS {h}u(nbr) \
              JOIN {src} {h}v ON {h}v.id = {h}u.nbr \
              WHERE {h}a.src = {owner_id} AND {h}a.dir = 'o'::\"char\" \
              AND {h}a.etype = {has_tid} \
              AND {} = ANY({})), '[]'::jsonb)",
            Self::type_of_id(&format!("{h}u.nbr")),
            Self::int_array(&tids)
        ))
    }

    fn cmp_sql(lhs: &str, op: CmpOp, rhs: &str) -> String {
        match op {
            CmpOp::Contains => format!("({lhs})::text ILIKE ('%' || ({rhs})::text || '%')"),
            CmpOp::Like => format!("({lhs})::text ~ ({rhs})::text"),
            _ => format!("({lhs}) {} ({rhs})", op.sql()),
        }
    }

    // ----------------------------------------------------------------------
    // Assembly
    // ----------------------------------------------------------------------

    fn subquery(&mut self, pats: &[Pat]) -> CResult<String> {
        let mut c = Compiler::nested(self);
        c.compile_patterns(pats)?;
        if c.from.is_empty() {
            // A branch with no joins is pure predicate; keep it truthy so the
            // disjunction still evaluates.
            let w = if c.wheres.is_empty() { "TRUE".into() } else { c.wheres.join(" AND ") };
            return Ok(format!("SELECT 1 WHERE {w}"));
        }
        Ok(format!("SELECT 1{}{}", c.from_sql(), c.where_sql()))
    }

    pub fn from_sql(&self) -> String {
        if self.from.is_empty() {
            return String::new();
        }
        let mut items = self.from.clone();
        // Inside a negation or disjunction every variable may come from the
        // enclosing query, so the first item can be a join clause with nothing
        // on its left. A lateral is a legal leading FROM item on its own.
        if let Some(first) = items.first_mut() {
            if let Some(rest) = first.strip_prefix("CROSS JOIN ") {
                *first = rest.to_string();
            }
        }
        format!(" FROM {}", items.join(" "))
    }

    pub fn where_sql(&self) -> String {
        if self.wheres.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", self.wheres.join(" AND "))
        }
    }

    /// SQL for a bound variable as a value (attributes project their value).
    pub fn var_value(&self, v: &str) -> CResult<String> {
        match self.binds.get(v) {
            Some(Bind::Attr { alias, .. }) => Ok(format!("{alias}.val")),
            Some(Bind::Instance { alias }) => Ok(format!("{alias}.id")),
            Some(Bind::Value { expr }) => Ok(expr.clone()),
            Some(Bind::Type { alias }) => Ok(format!("{alias}.name")),
            Some(Bind::Column { val, .. }) => Ok(val.clone()),
            None => Err(format!("'${v}' is not bound by any pattern")),
        }
    }

    /// SQL for a bound variable as an identity (used by write stages).
    pub fn var_id(&self, v: &str) -> CResult<String> {
        match self.binds.get(v) {
            Some(Bind::Attr { alias, .. }) | Some(Bind::Instance { alias }) => {
                Ok(format!("{alias}.id"))
            }
            Some(Bind::Value { expr }) => Ok(expr.clone()),
            Some(Bind::Type { alias }) => Ok(format!("{alias}.type_id")),
            Some(Bind::Column { val, id }) => Ok(id.clone().unwrap_or_else(|| val.clone())),
            None => Err(format!("'${v}' is not bound by any pattern")),
        }
    }

    pub fn is_attr(&self, v: &str) -> bool {
        matches!(self.binds.get(v), Some(Bind::Attr { .. }))
    }
}

// --------------------------------------------------------------------------
// Literals
// --------------------------------------------------------------------------

pub fn lit_str(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

pub fn lit_sql(l: &Lit) -> String {
    match l {
        Lit::Str(s) => lit_str(s),
        Lit::Int(i) => i.to_string(),
        Lit::Float(f) => format!("{f}"),
        Lit::Bool(b) => b.to_string(),
        Lit::DateTime(s) => {
            if s.contains('T') {
                format!("{}::timestamp", lit_str(&s.replace('T', " ")))
            } else {
                format!("{}::date", lit_str(s))
            }
        }
    }
}

// --------------------------------------------------------------------------
// Role helpers
// --------------------------------------------------------------------------

/// A role plus every role that specialises it, transitively (FR-009 / T031).
pub fn role_with_specialisations(role_id: i32) -> Vec<i32> {
    Spi::connect(|client| {
        client
            .select(
                "WITH RECURSIVE spec(role_id) AS (
                     SELECT role_id FROM og_catalog.role WHERE role_id = $1
                     UNION
                     SELECT c.role_id FROM og_catalog.role c
                       JOIN spec ON c.parent_role_id = spec.role_id
                 ) SELECT role_id FROM spec",
                None,
                &[role_id.into()],
            )
            .expect("role specialisation lookup failed")
            .filter_map(|r| r.get::<i32>(1).ok().flatten())
            .collect()
    })
}

/// Every role declared anywhere on a relation type's line of inheritance.
pub fn all_roles_of(rel_tid: i32) -> Vec<i32> {
    let mut line = labeling::og_supertypes(rel_tid);
    line.extend(labeling::og_subtypes(rel_tid));
    line.sort_unstable();
    line.dedup();
    Spi::connect(|client| {
        client
            .select(
                "SELECT role_id FROM og_catalog.role WHERE rel_type_id = ANY($1)",
                None,
                &[line.into()],
            )
            .expect("role lookup failed")
            .filter_map(|r| r.get::<i32>(1).ok().flatten())
            .collect()
    })
}

// --------------------------------------------------------------------------
// Connected components
// --------------------------------------------------------------------------

/// Variables a pattern touches, including inside negations and disjunctions.
fn vars_of(p: &Pat, out: &mut Vec<String>) {
    fn expr_vars(e: &Expr, out: &mut Vec<String>) {
        match e {
            Expr::Var(v) => out.push(v.clone()),
            Expr::Attr(v, _) => out.push(v.clone()),
            Expr::Bin(a, _, b) => {
                expr_vars(a, out);
                expr_vars(b, out);
            }
            Expr::Neg(a) => expr_vars(a, out),
            Expr::Call(_, args) => args.iter().for_each(|a| expr_vars(a, out)),
            Expr::Lit(_) => {}
        }
    }
    match p {
        Pat::Isa { var, .. } | Pat::IsaAnon { var, .. } | Pat::Sub { var, .. } | Pat::Label { var, .. } => {
            out.push(var.clone())
        }
        Pat::Has { owner, val, .. } => {
            out.push(owner.clone());
            match val {
                HasVal::Var(v) => out.push(v.clone()),
                HasVal::Cmp(_, e) => expr_vars(e, out),
                HasVal::Any => {}
            }
        }
        Pat::Role { rel, player, .. } => {
            out.push(rel.clone());
            out.push(player.clone());
        }
        Pat::Cmp { lhs, rhs, .. } => {
            expr_vars(lhs, out);
            expr_vars(rhs, out);
        }
        Pat::Let { var, expr } => {
            out.push(var.clone());
            expr_vars(expr, out);
        }
        Pat::Not(inner) => inner.iter().for_each(|q| vars_of(q, out)),
        Pat::Or(branches) => branches.iter().flatten().for_each(|q| vars_of(q, out)),
    }
}

/// Split a conjunction into groups that share no variables.
///
/// A `match` listing sixteen independently-pinned cities is one conjunction of
/// sixteen unrelated patterns. Handed to the planner as one flat join it has
/// 16! orderings, GEQO picks badly, and the query builds a cross product before
/// any name filter applies. Compiling each group as its own subquery removes
/// the choice entirely — there is nothing to get wrong between groups, and
/// `from_collapse_limit` keeps PostgreSQL from flattening them back together.
pub fn components(pats: &[Pat]) -> Vec<Vec<Pat>> {
    let n = pats.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut Vec<usize>, mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }

    let mut owner: HashMap<String, usize> = HashMap::new();
    for (i, p) in pats.iter().enumerate() {
        let mut vs = Vec::new();
        vars_of(p, &mut vs);
        for v in vs {
            match owner.get(&v) {
                Some(&j) => {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    if a != b {
                        parent[a] = b;
                    }
                }
                None => {
                    owner.insert(v, i);
                }
            }
        }
    }

    let mut groups: Vec<(usize, Vec<Pat>)> = Vec::new();
    for (i, p) in pats.iter().enumerate() {
        let root = find(&mut parent, i);
        match groups.iter_mut().find(|(r, _)| *r == root) {
            Some((_, g)) => g.push(p.clone()),
            None => groups.push((root, vec![p.clone()])),
        }
    }
    groups.into_iter().map(|(_, g)| g).collect()
}

#[cfg(test)]
mod unit {
    use super::*;

    fn isa(v: &str, t: &str) -> Pat {
        Pat::Isa { var: v.into(), ty: t.into(), exact: false }
    }
    fn has(o: &str, a: &str, lit: &str) -> Pat {
        Pat::Has {
            owner: o.into(),
            attr: Some(a.into()),
            val: HasVal::Cmp(CmpOp::Eq, Expr::Lit(Lit::Str(lit.into()))),
        }
    }

    #[test]
    fn independent_variables_split_into_groups() {
        let pats = vec![
            isa("a", "city"),
            has("a", "name", "Austin"),
            isa("b", "city"),
            has("b", "name", "Boston"),
        ];
        let g = components(&pats);
        assert_eq!(g.len(), 2, "two unrelated cities must not share a join tree");
        assert_eq!(g[0].len(), 2);
        assert_eq!(g[1].len(), 2);
    }

    #[test]
    fn shared_variables_stay_together() {
        let pats = vec![
            isa("a", "city"),
            has("a", "name", "Austin"),
            Pat::Role { rel: "r".into(), role: Some("location".into()), player: "a".into() },
            isa("r", "locating"),
            Pat::Role { rel: "r".into(), role: Some("located".into()), player: "b".into() },
            isa("b", "user"),
        ];
        assert_eq!(components(&pats).len(), 1);
    }
}
