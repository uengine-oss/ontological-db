//! Cypher → SQL compiler — spec 003 FR-010..FR-016.
//!
//! The output is ordinary SQL over ordinary relations. That is the whole point:
//! PostgreSQL's cost-based optimiser gets to choose the join order, the scan
//! methods and the parallelism for the graph pattern, using real statistics on
//! real tables. Apache AGE hides the pattern inside an opaque function pipeline
//! and forfeits all of that.
//!
//! Label predicates are resolved at *compile* time through the interval index
//! (spec 002), so a `MATCH (v:Vehicle)` never pays a per-row hierarchy walk.

use super::ast::*;
use super::views;
use crate::catalog::types;
use pgrx::prelude::*;
use std::collections::HashMap;

/// The bound jsonb parameter holding user `$params`.
pub const PARAM: &str = "$1";

#[derive(Debug, Clone)]
pub enum Bind {
    Node { alias: String, tid: Option<i32> },
    Rel { alias: Option<String>, tid: Option<i32>, id_expr: String },
    Path { hops_expr: String },
    Scalar { sql: String },
}

pub struct Compiler {
    pub graph: String,
    pub gid: i32,
    binds: HashMap<String, Bind>,
    from: Vec<String>,
    wheres: Vec<String>,
    ctes: Vec<String>,
    n: usize,
    /// Edge-id expressions of the single hops joined so far in the current
    /// MATCH clause — see `begin_match_clause`.
    rel_ids: Vec<String>,
    /// Emitted for EXPLAIN-style diagnostics and the agent surface (spec 008).
    pub notes: Vec<String>,
}

pub struct Compiled {
    pub sql: String,
    pub columns: Vec<String>,
}

type CResult<T> = Result<T, String>;

impl Compiler {
    pub fn new(graph: &str) -> Self {
        Compiler {
            graph: graph.to_string(),
            gid: types::graph_id(graph),
            binds: HashMap::new(),
            from: Vec::new(),
            wheres: Vec::new(),
            ctes: Vec::new(),
            n: 0,
            rel_ids: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// Start a new MATCH clause.
    ///
    /// Cypher matches relationships *isomorphically*: one MATCH clause may not
    /// traverse the same relationship twice, which is what stops
    /// `(a)-[:ACTED_IN]->(m)<-[:ACTED_IN]-(b)` from returning `a` as their own
    /// co-actor. The rule is scoped to the clause, so the set of hops resets
    /// here. Variable-length hops are excluded: `og_vlp` already walks trails.
    pub fn begin_match_clause(&mut self) {
        self.rel_ids.clear();
    }

    fn fresh(&mut self, p: &str) -> String {
        self.n += 1;
        format!("{p}{}", self.n)
    }

    pub fn binding(&self, v: &str) -> Option<&Bind> {
        self.binds.get(v)
    }

    pub fn push_where(&mut self, sql: String) {
        self.wheres.push(sql);
    }

    /// Variables bound so far, in a stable order.
    pub fn bound_vars(&self) -> Vec<String> {
        let mut v: Vec<String> = self.binds.keys().cloned().collect();
        v.sort();
        v
    }

    /// `build_select` for callers outside this module (the write path needs to
    /// materialise bindings before applying mutations).
    pub fn build_select_pub(&mut self, proj: &Projection) -> CResult<Compiled> {
        self.build_select(proj)
    }

    // ----------------------------------------------------------------------
    // Read query compilation
    // ----------------------------------------------------------------------

    pub fn compile_read(&mut self, q: &Query) -> CResult<Compiled> {
        let mut projection: Option<&Projection> = None;

        for clause in &q.clauses {
            match clause {
                Clause::Match { patterns, optional, where_ } => {
                    self.begin_match_clause();
                    for p in patterns {
                        self.compile_pattern(p, *optional)?;
                    }
                    if let Some(w) = where_ {
                        let sql = self.expr(w, None)?;
                        self.wheres.push(sql);
                    }
                }
                Clause::Unwind { expr, alias } => {
                    let src = self.expr(expr, None)?;
                    let a = self.fresh("uw");
                    self.from.push(format!(
                        "CROSS JOIN LATERAL jsonb_array_elements(({src})::jsonb) AS {a}(v)"
                    ));
                    self.binds
                        .insert(alias.clone(), Bind::Scalar { sql: format!("{a}.v") });
                }
                Clause::Return(p) => projection = Some(p),
                Clause::With { .. } => {
                    return Err("WITH is not supported in this release; split the query or use a \
                                subquery on the SQL side"
                        .into())
                }
                _ => return Err("this clause is only valid in a write query".into()),
            }
        }

        let proj = projection.ok_or("a read query must end with RETURN")?;
        self.build_select(proj)
    }

    fn build_select(&mut self, proj: &Projection) -> CResult<Compiled> {
        let mut cols = Vec::new();
        let mut names = Vec::new();
        let mut group_by = Vec::new();
        let has_agg = proj.items.iter().any(|i| i.expr.is_aggregate());

        for (i, item) in proj.items.iter().enumerate() {
            if matches!(item.expr, Expr::Star) {
                let vars: Vec<String> = self.binds.keys().cloned().collect();
                for v in vars {
                    let sql = self.var_value(&v)?;
                    cols.push(format!("{sql} AS c{}", cols.len()));
                    names.push(v);
                }
                continue;
            }
            let sql = self.expr(&item.expr, None)?;
            let name = item.alias.clone().unwrap_or_else(|| item.expr.default_alias());
            if has_agg && !item.expr.is_aggregate() {
                group_by.push(sql.clone());
            }
            cols.push(format!("{sql} AS c{i}"));
            names.push(name);
        }

        // ORDER BY may reference a RETURN alias rather than a pattern variable,
        // which is standard Cypher — resolve those against the projection first.
        let alias_sql: std::collections::HashMap<String, String> = proj
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                let name = item.alias.clone().unwrap_or_else(|| item.expr.default_alias());
                cols.get(i).map(|c| {
                    let expr = c.rsplit_once(" AS ").map(|(e, _)| e.to_string()).unwrap_or_else(|| c.clone());
                    (name, expr)
                })
            })
            .collect();

        let mut order_cols = Vec::new();
        let mut order_by = Vec::new();
        for (i, o) in proj.order.iter().enumerate() {
            let sql = match &o.expr {
                Expr::Var(v) if !self.binds.contains_key(v) => alias_sql
                    .get(v)
                    .cloned()
                    .ok_or_else(|| format!("ORDER BY refers to unknown name '{v}'"))?,
                _ => self.expr(&o.expr, None)?,
            };
            if has_agg && !o.expr.is_aggregate() && !group_by.contains(&sql) {
                group_by.push(sql.clone());
            }
            order_cols.push(format!("{sql} AS o{i}"));
            order_by.push(format!("o{i}{}", if o.desc { " DESC" } else { "" }));
        }

        let from = if self.from.is_empty() {
            String::new()
        } else {
            format!(" FROM {}", self.from.join("\n  "))
        };
        let where_clause = if self.wheres.is_empty() {
            String::new()
        } else {
            format!("\n WHERE {}", self.wheres.join("\n   AND "))
        };
        let group = if group_by.is_empty() {
            String::new()
        } else {
            format!("\n GROUP BY {}", group_by.join(", "))
        };

        let select_list = cols
            .iter()
            .chain(order_cols.iter())
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let distinct = if proj.distinct { "DISTINCT " } else { "" };

        let inner = format!(
            "SELECT {distinct}{select_list}{from}{where_clause}{group}"
        );

        let json_pairs: Vec<String> = names
            .iter()
            .enumerate()
            .map(|(i, n)| format!("{}, t.c{i}", sql_str(n)))
            .collect();

        let order = if order_by.is_empty() {
            String::new()
        } else {
            format!("\n ORDER BY {}", order_by.iter().map(|o| format!("t.{o}")).collect::<Vec<_>>().join(", "))
        };
        let limit = match &proj.limit {
            Some(e) => format!("\n LIMIT {}", self.expr(e, Some("int8"))?),
            None => String::new(),
        };
        let skip = match &proj.skip {
            Some(e) => format!("\n OFFSET {}", self.expr(e, Some("int8"))?),
            None => String::new(),
        };

        let with = if self.ctes.is_empty() {
            String::new()
        } else {
            format!("WITH RECURSIVE {}\n", self.ctes.join(",\n"))
        };

        let sql = format!(
            "{with}SELECT jsonb_build_object({}) AS row FROM (\n{inner}\n) t{order}{limit}{skip}",
            json_pairs.join(", ")
        );

        Ok(Compiled { sql, columns: names })
    }

    // ----------------------------------------------------------------------
    // Patterns
    // ----------------------------------------------------------------------

    pub fn compile_pattern(&mut self, p: &Pattern, optional: bool) -> CResult<()> {
        if optional {
            self.notes.push("OPTIONAL MATCH compiled as LEFT JOIN LATERAL".into());
        }
        let mut prev_node: Option<String> = None;
        let mut pending_rel: Option<&RelPat> = None;
        let mut hop_exprs: Vec<String> = Vec::new();

        for elem in &p.elems {
            match elem {
                PatElem::Node(np) => {
                    let alias = self.bind_node(np, optional && prev_node.is_some())?;
                    if let (Some(prev), Some(rel)) = (prev_node.clone(), pending_rel.take()) {
                        let hop = self.join_rel(&prev, rel, &alias, optional)?;
                        hop_exprs.push(hop);
                    }
                    prev_node = Some(alias);
                }
                PatElem::Rel(rp) => pending_rel = Some(rp),
            }
        }

        if let Some(pv) = &p.path_var {
            let expr = if hop_exprs.is_empty() {
                "'[]'::jsonb".to_string()
            } else {
                format!("jsonb_build_array({})", hop_exprs.join(", "))
            };
            self.binds.insert(pv.clone(), Bind::Path { hops_expr: expr });
        }
        Ok(())
    }

    fn resolve_label(&mut self, labels: &[String]) -> CResult<Option<i32>> {
        if labels.is_empty() {
            return Ok(None);
        }
        if labels.len() > 1 {
            return Err(format!(
                "multi-label patterns (:{}) are not supported yet; declare a common supertype \
                 instead — that is what the type system is for",
                labels.join(":")
            ));
        }
        let name = &labels[0];
        match types::try_type_id(self.gid, name) {
            Some(t) => Ok(Some(t)),
            None => {
                let near = types::nearest_type_names(self.gid, name);
                Err(if near.is_empty() {
                    format!("unknown label '{name}' in graph '{}'", self.graph)
                } else {
                    format!(
                        "unknown label '{name}' in graph '{}'. did you mean: {}",
                        self.graph,
                        near.join(", ")
                    )
                })
            }
        }
    }

    fn bind_node(&mut self, np: &NodePat, optional: bool) -> CResult<String> {
        // Re-using an existing variable: reuse its alias, do not join again.
        if let Some(v) = &np.var {
            if let Some(Bind::Node { alias, tid }) = self.binds.get(v).cloned() {
                if !np.labels.is_empty() {
                    let want = self.resolve_label(&np.labels)?;
                    if let (Some(w), Some(t)) = (want, tid) {
                        if w != t && !crate::catalog::labeling::og_is_subtype(t, w) {
                            self.wheres.push("false".into());
                        }
                    }
                }
                self.push_prop_filters(&alias, tid, &np.props)?;
                return Ok(alias);
            }
        }

        let tid = self.resolve_label(&np.labels)?;
        let alias = self.fresh("n");
        let rel = match tid {
            Some(t) => views::ensure_view(t, false),
            None => "og_data.og_node".to_string(),
        };

        let join = if self.from.is_empty() {
            format!("{rel} {alias}")
        } else if optional {
            format!("LEFT JOIN {rel} {alias} ON true")
        } else {
            format!("CROSS JOIN {rel} {alias}")
        };
        self.from.push(join);

        if let Some(v) = &np.var {
            self.binds.insert(v.clone(), Bind::Node { alias: alias.clone(), tid });
        }
        self.push_prop_filters(&alias, tid, &np.props)?;
        Ok(alias)
    }

    fn push_prop_filters(
        &mut self,
        alias: &str,
        tid: Option<i32>,
        props: &[(String, Expr)],
    ) -> CResult<()> {
        for (k, v) in props {
            let (lhs, ty) = self.prop_sql(alias, tid, k);
            let rhs = self.expr(v, ty.as_deref())?;
            self.wheres.push(format!("{lhs} = {rhs}"));
        }
        Ok(())
    }

    /// Join a relationship. Returns a jsonb expression describing the hop, used
    /// when the pattern binds a path variable.
    fn join_rel(
        &mut self,
        from_alias: &str,
        rel: &RelPat,
        to_alias: &str,
        optional: bool,
    ) -> CResult<String> {
        let etypes = if rel.types.is_empty() {
            None
        } else {
            let mut ids = Vec::new();
            for t in &rel.types {
                let tid = self.resolve_label(std::slice::from_ref(t))?;
                let tid = tid.unwrap();
                ids.extend(crate::catalog::labeling::og_subtypes(tid));
            }
            ids.sort_unstable();
            ids.dedup();
            Some(ids)
        };
        let etype_pred = match &etypes {
            Some(ids) => format!(
                "ARRAY[{}]::int4[]",
                ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")
            ),
            None => "NULL::int4[]".to_string(),
        };
        let dir_lit = match rel.dir {
            Dir::Out => "'o'",
            Dir::In => "'i'",
            Dir::Both => "'b'",
        };

        if let Some((min, max)) = rel.range {
            let w = self.fresh("vl");
            let joiner = if optional { "LEFT JOIN LATERAL" } else { "CROSS JOIN LATERAL" };
            let on = if optional { " ON true" } else { "" };
            self.from.push(format!(
                "{joiner} og_vlp({from_alias}.id, {etype_pred}, {dir_lit}::\"char\", {min}, {max}) {w}{on}"
            ));
            self.wheres.push(format!("{to_alias}.id = {w}.node"));
            if let Some(v) = &rel.var {
                self.binds.insert(
                    v.clone(),
                    Bind::Path { hops_expr: format!("to_jsonb({w}.path)") },
                );
            }
            return Ok(format!("to_jsonb({w}.path)"));
        }

        let a = self.fresh("adj");
        let u = self.fresh("u");
        let joiner = if optional { "LEFT JOIN LATERAL" } else { "CROSS JOIN LATERAL" };
        let on = if optional { " ON true" } else { "" };
        let type_pred = match &etypes {
            Some(_) => format!(" AND {a}.etype = ANY({etype_pred})"),
            None => String::new(),
        };
        let dir_pred = match rel.dir {
            Dir::Both => format!("{a}.dir IN ('o','i')"),
            _ => format!("{a}.dir = {dir_lit}::\"char\""),
        };
        self.from.push(format!(
            "{joiner} (SELECT u.nbr, u.eid FROM og_data.og_adj {a}, \
             LATERAL unnest({a}.nbr, {a}.eid) AS u(nbr, eid) \
             WHERE {a}.src = {from_alias}.id AND {dir_pred}{type_pred}) {u}{on}"
        ));
        self.wheres.push(format!("{to_alias}.id = {u}.nbr"));

        if !optional {
            for other in std::mem::take(&mut self.rel_ids) {
                self.wheres.push(format!("{u}.eid <> {other}"));
                self.rel_ids.push(other);
            }
            self.rel_ids.push(format!("{u}.eid"));
        }

        if let Some(v) = &rel.var {
            let rtid = if rel.types.len() == 1 {
                types::try_type_id(self.gid, &rel.types[0])
            } else {
                None
            };
            if !rel.props.is_empty() || rtid.is_some() {
                if let Some(t) = rtid {
                    let ev = views::ensure_view(t, true);
                    let ea = self.fresh("e");
                    self.from.push(format!("JOIN {ev} {ea} ON {ea}.id = {u}.eid"));
                    self.binds.insert(
                        v.clone(),
                        Bind::Rel { alias: Some(ea.clone()), tid: Some(t), id_expr: format!("{u}.eid") },
                    );
                    self.push_rel_prop_filters(&ea, Some(t), &rel.props)?;
                    return Ok(format!("to_jsonb({u}.eid)"));
                }
            }
            self.binds
                .insert(v.clone(), Bind::Rel { alias: None, tid: None, id_expr: format!("{u}.eid") });
        } else if !rel.props.is_empty() {
            let rtid = if rel.types.len() == 1 {
                types::try_type_id(self.gid, &rel.types[0])
            } else {
                None
            };
            if let Some(t) = rtid {
                let ev = views::ensure_view(t, true);
                let ea = self.fresh("e");
                self.from.push(format!("JOIN {ev} {ea} ON {ea}.id = {u}.eid"));
                self.push_rel_prop_filters(&ea, Some(t), &rel.props)?;
            }
        }
        Ok(format!("to_jsonb({u}.eid)"))
    }

    fn push_rel_prop_filters(
        &mut self,
        alias: &str,
        tid: Option<i32>,
        props: &[(String, Expr)],
    ) -> CResult<()> {
        for (k, v) in props {
            let (lhs, ty) = self.prop_sql(alias, tid, k);
            let rhs = self.expr(v, ty.as_deref())?;
            self.wheres.push(format!("{lhs} = {rhs}"));
        }
        Ok(())
    }

    // ----------------------------------------------------------------------
    // Expressions
    // ----------------------------------------------------------------------

    /// SQL for a property access, plus the column's SQL type when known.
    pub fn prop_sql(&self, alias: &str, tid: Option<i32>, prop: &str) -> (String, Option<String>) {
        if prop == "id" || prop == "_id" {
            return (format!("{alias}.id"), Some("int8".into()));
        }
        if let Some(t) = tid {
            let props = views::view_properties(t);
            if let Some((col, dt)) = props.get(prop) {
                return (format!("{alias}.{col}"), Some(dt.clone()));
            }
        }
        match tid {
            // Typed view: the property is not declared, so it can only live in
            // the extension payload.
            Some(_) => (format!("({alias}.__ext->>{})", sql_str(prop)), None),
            // Untyped variable: resolve through the catalog at run time.
            None => (format!("(og_node_json({alias}.id)->>{})", sql_str(prop)), None),
        }
    }

    fn var_value(&mut self, v: &str) -> CResult<String> {
        match self.binds.get(v).cloned() {
            Some(Bind::Node { alias, tid }) => Ok(self.node_json(&alias, tid)),
            Some(Bind::Rel { alias, tid, id_expr }) => Ok(match (alias, tid) {
                (Some(a), Some(t)) => self.rel_json(&a, t),
                _ => format!("og_edge_json({id_expr})"),
            }),
            Some(Bind::Path { hops_expr }) => Ok(hops_expr),
            Some(Bind::Scalar { sql }) => Ok(sql),
            None => Err(format!("variable '{v}' is not defined in this query")),
        }
    }

    fn node_json(&self, alias: &str, tid: Option<i32>) -> String {
        match tid {
            Some(t) => {
                let props = views::view_properties(t);
                let mut pairs = vec![
                    format!("'_id', {alias}.id"),
                    format!("'_type', og_type_name({alias}.type_id)"),
                ];
                for (name, (col, _)) in &props {
                    pairs.push(format!("{}, to_jsonb({alias}.{col})", sql_str(name)));
                }
                format!(
                    "(jsonb_strip_nulls(jsonb_build_object({})) || COALESCE({alias}.__ext,'{{}}'::jsonb))",
                    pairs.join(", ")
                )
            }
            None => format!("og_node_json({alias}.id)"),
        }
    }

    fn rel_json(&self, alias: &str, tid: i32) -> String {
        let props = views::view_properties(tid);
        let mut pairs = vec![
            format!("'_id', {alias}.id"),
            format!("'_type', og_type_name({alias}.type_id)"),
            format!("'_src', {alias}.src"),
            format!("'_dst', {alias}.dst"),
        ];
        for (name, (col, _)) in &props {
            pairs.push(format!("{}, to_jsonb({alias}.{col})", sql_str(name)));
        }
        format!(
            "(jsonb_strip_nulls(jsonb_build_object({})) || COALESCE({alias}.__ext,'{{}}'::jsonb))",
            pairs.join(", ")
        )
    }

    /// An element of a list or map literal, wrapped for `to_jsonb()`.
    ///
    /// String and NULL literals compile to bare SQL literals, which carry the
    /// `unknown` type until something resolves them — and `to_jsonb()` is
    /// polymorphic, so it cannot. `['Neo']` would fail to compile without the
    /// explicit type here.
    fn jsonb_arg(&mut self, e: &Expr) -> CResult<String> {
        Ok(match e {
            Expr::Lit(Lit::Null) => "'null'::jsonb".into(),
            Expr::Lit(Lit::Str(s)) => format!("to_jsonb({}::text)", sql_str(s)),
            _ => format!("to_jsonb({})", self.expr(e, None)?),
        })
    }

    pub fn expr(&mut self, e: &Expr, hint: Option<&str>) -> CResult<String> {
        Ok(match e {
            Expr::Lit(Lit::Int(i)) => i.to_string(),
            Expr::Lit(Lit::Float(f)) => format!("{f}::float8"),
            Expr::Lit(Lit::Bool(b)) => b.to_string(),
            Expr::Lit(Lit::Null) => "NULL".into(),
            Expr::Lit(Lit::Str(s)) => match hint {
                Some(t) if t != "text" => format!("{}::{t}", sql_str(s)),
                _ => sql_str(s),
            },
            Expr::Param(p) => {
                let base = format!("({PARAM} ->> {})", sql_str(p));
                match hint {
                    Some(t) if t != "text" => format!("{base}::{t}"),
                    _ => base,
                }
            }
            Expr::Var(v) => self.var_value(v)?,
            Expr::Prop(base, p) => {
                let Expr::Var(v) = &**base else {
                    return Err("property access is only supported on pattern variables".into());
                };
                match self.binds.get(v).cloned() {
                    Some(Bind::Node { alias, tid }) => self.prop_sql(&alias, tid, p).0,
                    Some(Bind::Rel { alias: Some(a), tid, .. }) => self.prop_sql(&a, tid, p).0,
                    Some(Bind::Rel { id_expr, .. }) => {
                        format!("(og_edge_json({id_expr}) ->> {})", sql_str(p))
                    }
                    Some(Bind::Scalar { sql }) => format!("({sql} ->> {})", sql_str(p)),
                    Some(Bind::Path { .. }) => {
                        return Err(format!("'{v}' is a path; use length({v}) or nodes({v})"))
                    }
                    None => return Err(format!("variable '{v}' is not defined in this query")),
                }
            }
            Expr::Not(a) => format!("(NOT {})", self.expr(a, Some("bool"))?),
            Expr::Neg(a) => format!("(- {})", self.expr(a, None)?),
            Expr::IsNull(a, want_null) => {
                let s = self.expr(a, None)?;
                if *want_null {
                    format!("({s} IS NULL)")
                } else {
                    format!("({s} IS NOT NULL)")
                }
            }
            Expr::Binary(op, l, r) => self.binary(*op, l, r)?,
            // Against an array column the list is that array, not a jsonb one:
            // `MERGE (a)-[:ACTED_IN {roles:['Neo']}]->(m)` has to compare
            // `text[] = text[]`.
            Expr::List(xs) if hint.is_some_and(|t| t.ends_with("[]")) => {
                let ty = hint.unwrap();
                let elem = &ty[..ty.len() - 2];
                let items: Vec<String> =
                    xs.iter().map(|x| self.expr(x, Some(elem))).collect::<CResult<_>>()?;
                format!("ARRAY[{}]::{ty}", items.join(", "))
            }
            Expr::List(xs) => {
                let items: Vec<String> =
                    xs.iter().map(|x| self.jsonb_arg(x)).collect::<CResult<_>>()?;
                format!("jsonb_build_array({})", items.join(", "))
            }
            Expr::Map(kv) => {
                let mut pairs = Vec::new();
                for (k, v) in kv {
                    pairs.push(format!("{}, {}", sql_str(k), self.jsonb_arg(v)?));
                }
                format!("jsonb_build_object({})", pairs.join(", "))
            }
            Expr::Case { operand, whens, else_ } => {
                let head = match operand {
                    Some(o) => format!("CASE {} ", self.expr(o, None)?),
                    None => "CASE ".into(),
                };
                let mut body = String::new();
                for (c, v) in whens {
                    body.push_str(&format!(
                        "WHEN {} THEN {} ",
                        self.expr(c, None)?,
                        self.expr(v, None)?
                    ));
                }
                let tail = match else_ {
                    Some(e) => format!("ELSE {} ", self.expr(e, None)?),
                    None => String::new(),
                };
                format!("({head}{body}{tail}END)")
            }
            Expr::Func { name, args, distinct } => self.func(name, args, *distinct)?,
            Expr::Star => "*".into(),
        })
    }

    fn binary(&mut self, op: BinOp, l: &Expr, r: &Expr) -> CResult<String> {
        // Type-directed coercion: comparing a typed column to a parameter or
        // literal casts the right-hand side to the column's type, so the index
        // on that column stays usable.
        // Each side is compiled with the *other* side's type as the hint, so a
        // parameter compared against a typed column is cast to that column's
        // type and the index on it stays usable.
        let lhint = self.type_of(r);
        let rhint = self.type_of(l);
        let ls = self.expr(l, lhint.as_deref())?;
        let rs = self.expr(r, rhint.as_deref())?;
        Ok(match op {
            BinOp::Add => format!("({ls} + {rs})"),
            BinOp::Sub => format!("({ls} - {rs})"),
            BinOp::Mul => format!("({ls} * {rs})"),
            BinOp::Div => format!("({ls} / NULLIF({rs}, 0))"),
            BinOp::Mod => format!("(({ls})::numeric % NULLIF(({rs})::numeric, 0))"),
            BinOp::Pow => format!("(({ls})::float8 ^ ({rs})::float8)"),
            BinOp::Eq => format!("({ls} IS NOT DISTINCT FROM {rs})"),
            BinOp::Ne => format!("({ls} IS DISTINCT FROM {rs})"),
            BinOp::Lt => format!("({ls} < {rs})"),
            BinOp::Le => format!("({ls} <= {rs})"),
            BinOp::Gt => format!("({ls} > {rs})"),
            BinOp::Ge => format!("({ls} >= {rs})"),
            BinOp::And => format!("({ls} AND {rs})"),
            BinOp::Or => format!("({ls} OR {rs})"),
            BinOp::Xor => format!("(({ls}) <> ({rs}))"),
            BinOp::Concat => format!("(({ls})::text || ({rs})::text)"),
            BinOp::StartsWith => format!("(({ls})::text LIKE ({rs})::text || '%')"),
            BinOp::EndsWith => format!("(({ls})::text LIKE '%' || ({rs})::text)"),
            BinOp::Contains => format!("(strpos(({ls})::text, ({rs})::text) > 0)"),
            BinOp::Regex => format!("(({ls})::text ~ ({rs})::text)"),
            BinOp::In => format!("(({rs}) @> to_jsonb({ls}))"),
        })
    }

    /// The SQL type of an expression when we can determine it — used to drive
    /// parameter coercion.
    fn type_of(&self, e: &Expr) -> Option<String> {
        match e {
            Expr::Prop(base, p) => {
                let Expr::Var(v) = &**base else { return None };
                match self.binds.get(v) {
                    Some(Bind::Node { alias, tid }) => self.prop_sql(alias, *tid, p).1,
                    Some(Bind::Rel { alias: Some(a), tid, .. }) => self.prop_sql(a, *tid, p).1,
                    _ => None,
                }
            }
            Expr::Lit(Lit::Int(_)) => Some("int8".into()),
            Expr::Lit(Lit::Float(_)) => Some("float8".into()),
            Expr::Lit(Lit::Bool(_)) => Some("bool".into()),
            Expr::Lit(Lit::Str(_)) => Some("text".into()),
            _ => None,
        }
    }

    fn func(&mut self, name: &str, args: &[Expr], distinct: bool) -> CResult<String> {
        let lname = name.to_ascii_lowercase();
        let d = if distinct { "DISTINCT " } else { "" };

        // Aggregates
        match lname.as_str() {
            "count" => {
                return Ok(if matches!(args.first(), Some(Expr::Star)) || args.is_empty() {
                    "count(*)".into()
                } else {
                    format!("count({d}{})", self.expr(&args[0], None)?)
                })
            }
            "sum" | "avg" | "min" | "max" => {
                let a = self.expr(&args[0], None)?;
                return Ok(format!("{lname}({d}{a})"));
            }
            "collect" => {
                let a = self.expr(&args[0], None)?;
                return Ok(format!("jsonb_agg({d}{a})"));
            }
            "stdev" => {
                let a = self.expr(&args[0], None)?;
                return Ok(format!("stddev_samp(({a})::float8)"));
            }
            _ => {}
        }

        let mut a = Vec::new();
        for x in args {
            a.push(self.expr(x, None)?);
        }

        Ok(match lname.as_str() {
            "id" => {
                let Some(Expr::Var(v)) = args.first() else {
                    return Err("id() expects a pattern variable".into());
                };
                match self.binds.get(v) {
                    Some(Bind::Node { alias, .. }) => format!("{alias}.id"),
                    Some(Bind::Rel { alias: Some(al), .. }) => format!("{al}.id"),
                    Some(Bind::Rel { id_expr, .. }) => id_expr.clone(),
                    _ => return Err(format!("id() cannot be applied to '{v}'")),
                }
            }
            "labels" | "type" => {
                let Some(Expr::Var(v)) = args.first() else {
                    return Err(format!("{lname}() expects a pattern variable"));
                };
                match self.binds.get(v) {
                    Some(Bind::Node { alias, .. }) => format!("og_type_name({alias}.type_id)"),
                    Some(Bind::Rel { alias: Some(al), .. }) => format!("og_type_name({al}.type_id)"),
                    Some(Bind::Rel { id_expr, .. }) => format!("og_type_name(og_id_type({id_expr}))"),
                    _ => return Err(format!("{lname}() cannot be applied to '{v}'")),
                }
            }
            "length" | "size" => format!("jsonb_array_length({})", a[0]),
            "toupper" | "upper" => format!("upper(({})::text)", a[0]),
            "tolower" | "lower" => format!("lower(({})::text)", a[0]),
            "trim" => format!("btrim(({})::text)", a[0]),
            "substring" => {
                if a.len() == 3 {
                    format!("substring(({})::text from ({})::int + 1 for ({})::int)", a[0], a[1], a[2])
                } else {
                    format!("substring(({})::text from ({})::int + 1)", a[0], a[1])
                }
            }
            "replace" => format!("replace(({})::text, ({})::text, ({})::text)", a[0], a[1], a[2]),
            "split" => format!("to_jsonb(string_to_array(({})::text, ({})::text))", a[0], a[1]),
            "coalesce" => format!("coalesce({})", a.join(", ")),
            "abs" => format!("abs({})", a[0]),
            "ceil" => format!("ceil(({})::float8)", a[0]),
            "floor" => format!("floor(({})::float8)", a[0]),
            "round" => format!("round(({})::numeric)", a[0]),
            "sqrt" => format!("sqrt(({})::float8)", a[0]),
            "rand" => "random()".into(),
            "tostring" => format!("({})::text", a[0]),
            "tointeger" => format!("({})::int8", a[0]),
            "tofloat" => format!("({})::float8", a[0]),
            "timestamp" => "extract(epoch from now())::int8".into(),
            "datetime" => "now()".into(),
            "exists" => format!("({} IS NOT NULL)", a[0]),
            "keys" => format!("to_jsonb(ARRAY(SELECT jsonb_object_keys({})))", a[0]),
            "element_at" => format!("({} -> ({})::int)", a[0], a[1]),
            "nodes" | "relationships" => a[0].clone(),
            // spec 004 — vector surface
            "vector.similarity" | "similarity" => {
                format!("(1 - (({})::vector <=> ({})::vector))", a[0], a[1])
            }
            "vector.distance" => format!("(({})::vector <=> ({})::vector)", a[0], a[1]),
            "vector.l2" => format!("(({})::vector <-> ({})::vector)", a[0], a[1]),
            other => {
                return Err(format!(
                    "unknown function '{other}'. supported: count, sum, avg, min, max, collect, \
                     id, labels, type, length, size, toUpper, toLower, trim, substring, replace, \
                     split, coalesce, abs, ceil, floor, round, sqrt, rand, toString, toInteger, \
                     toFloat, timestamp, exists, keys, vector.similarity, vector.distance"
                ))
            }
        })
    }
}

/// SQL string literal with proper escaping.
pub fn sql_str(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}
