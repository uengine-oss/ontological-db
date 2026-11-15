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
use std::collections::HashMap;

/// The bound jsonb parameter holding user `$params`.
pub const PARAM: &str = "$1";

/// Is enumerating trails to `max` hops more work than the answer is big?
///
/// Reachability is not free: `og_reach` is a Rust set-returning function, so
/// unlike `og_vlp` it does not inline into the surrounding plan and it pays SPI
/// setup per level — measured at a few tenths of a millisecond. Below the point
/// where trail enumeration explodes, that overhead is the whole query, and
/// rewriting makes it slower. Above it, the same overhead is invisible next to
/// a factor of a thousand.
///
/// The crossover is where the number of walks passes the number of nodes there
/// are to find: `Σ degreeⁱ > |V|`. Both terms come from the planner's own
/// statistics — a catalog lookup, not a scan — so this costs nothing to ask.
/// An unanalysed database has no statistics to answer with, and falls back to
/// depth alone.
fn prefer_reachability(max: u32) -> bool {
    /// Estimated walks past which `og_reach`'s fixed cost is repaid.
    ///
    /// Fitted to measurement rather than derived, and deliberately low. The two
    /// failure modes are not symmetric: enumerating when we should not have
    /// runs out of time or memory — 2.7 s at twenty hops on a lattice, 90 s at
    /// thirty — while reaching when we should not have costs a bounded fraction
    /// of a millisecond. A rule this cheap should err toward the bounded loss.
    const WALKS: f64 = 512.0;
    /// Degree cannot be estimated on an unanalysed table, so depth alone decides.
    const DEEP: u32 = 4;

    let est = crate::spiu::two::<f32, f32>(
        "SELECT (SELECT reltuples FROM pg_class WHERE oid = 'og_data.og_node'::regclass),
                (SELECT reltuples FROM pg_class WHERE oid = 'og_data.og_edge'::regclass)",
        &[],
    );
    let (nodes, edges) = match est {
        Ok((Some(n), Some(e))) if n > 0.0 && e > 0.0 => (n as f64, e as f64),
        _ => return max >= DEEP,
    };
    // Average out-degree over the whole graph. A per-relation-type figure would
    // be sharper, but this decision only has to be right about an order of
    // magnitude, and it must not cost a scan to make.
    let degree = (edges / nodes).max(1.0);

    // An earlier version compared the walk count against |V| instead, on the
    // reasoning that enumeration is affordable while it produces fewer rows
    // than there are nodes to find. That is wrong wherever many walks land on
    // the same node: on a 1000x1000 lattice ten hops is 2,046 walks against a
    // million nodes — "affordable" by that rule — but only 66 nodes are
    // reachable, and enumerating cost 3.83 ms against 0.30 ms. Degree alone
    // cannot see that overlap, so the rule no longer pretends to; it asks only
    // whether enough walks are coming to pay for the switch.
    let mut walks = 0.0f64;
    let mut level = 1.0f64;
    for _ in 0..max {
        level *= degree;
        walks += level;
        if walks > WALKS || !walks.is_finite() {
            return true;
        }
    }
    false
}

/// Aggregates whose value does not change when a row is duplicated, provided
/// their argument is spelled `DISTINCT`. `min`/`max` need no such qualifier.
fn blind_expr(e: &Expr) -> bool {
    match e {
        Expr::Func { name, args, distinct } => {
            let n = name.to_ascii_lowercase();
            let ok = match n.as_str() {
                "min" | "max" => true,
                "count" | "collect" => *distinct && !matches!(args.first(), Some(Expr::Star)),
                // sum/avg/stdev and anything user-defined: duplicates move them.
                _ => !e.is_aggregate(),
            };
            ok && args.iter().all(blind_expr)
        }
        Expr::Binary(_, a, b) => blind_expr(a) && blind_expr(b),
        Expr::Not(a) | Expr::Neg(a) | Expr::IsNull(a, _) => blind_expr(a),
        Expr::List(xs) => xs.iter().all(blind_expr),
        Expr::Map(kv) => kv.iter().all(|(_, v)| blind_expr(v)),
        _ => !e.is_aggregate(),
    }
}

#[derive(Debug, Clone)]
pub enum Bind {
    Node { alias: String, tid: Option<i32> },
    Rel { alias: Option<String>, tid: Option<i32>, id_expr: String },
    /// A path variable: `hops_expr` is the path's value — jsonb
    /// `{"_path": true, "nodes": [...], "relationships": [...]}`, or null when an
    /// OPTIONAL MATCH found none.
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
    /// Set while an OPTIONAL MATCH is being compiled. See `OptionalScope`.
    optional: Option<OptionalScope>,
    /// True when the query cannot observe how many *paths* reach a node, only
    /// which nodes are reached — see [`Compiler::multiplicity_blind`].
    reachability_only: bool,
    /// The type each node alias was bound to — a virtual relationship joins on
    /// the endpoints' columns, so it has to know whose columns they are.
    alias_tid: HashMap<String, Option<i32>>,
    /// Node aliases whose label set can match nothing.
    unmatchable: std::collections::HashSet<String>,
    /// Values carried through `WITH` are jsonb. What they were before — the
    /// type of a node, the SQL type of a number or a date — is kept here by
    /// name, so `WITH o … RETURN sum(o.qty * o.price)` still multiplies
    /// numbers and `WITH c MATCH (c)<-[:R]-(o)` still knows what `c` is.
    carried_tid: HashMap<String, i32>,
    scalar_types: HashMap<String, String>,
    /// The `og_vlp` alias of the variable-length hop `join_rel` just joined —
    /// a path over it needs the nodes along the way, not only its ends.
    last_var_hop: Option<String>,
    /// Names carried through WITH whose value is a list (`collect(x) AS xs`),
    /// so that `xs + ys` concatenates rather than adds.
    carried_lists: std::collections::HashSet<String>,
    /// The names the last `CALL … [YIELD …]` brought into scope: what a query
    /// that is only a CALL returns.
    last_yields: Vec<String>,
}

/// The joins and predicates of one OPTIONAL MATCH.
///
/// An optional pattern's predicates cannot go in `WHERE`: `A LEFT JOIN B ON
/// true WHERE b.x = a.y` throws away exactly the rows the LEFT JOIN was there
/// to keep, turning OPTIONAL MATCH back into MATCH. They belong in the `ON` of
/// the join that introduced the alias they constrain, so they are collected
/// here and placed when the clause closes.
#[derive(Default)]
struct OptionalScope {
    /// (alias, index into `from`) for each join this clause added, in order.
    joins: Vec<(String, usize)>,
    preds: Vec<String>,
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
            optional: None,
            reachability_only: false,
            alias_tid: HashMap::new(),
            unmatchable: std::collections::HashSet::new(),
            carried_tid: HashMap::new(),
            scalar_types: HashMap::new(),
            last_var_hop: None,
            carried_lists: std::collections::HashSet::new(),
            last_yields: Vec::new(),
        }
    }

    /// A name about to be bound to a fresh scalar (UNWIND, CALL … YIELD, a
    /// comprehension binder) no longer carries what an earlier `WITH` said
    /// about it. Returns what was there, for a binder to restore.
    fn forget_carried(&mut self, name: &str) -> (Option<i32>, Option<String>) {
        self.carried_lists.remove(name);
        (self.carried_tid.remove(name), self.scalar_types.remove(name))
    }

    fn restore_carried(&mut self, name: &str, saved: (Option<i32>, Option<String>)) {
        if let Some(t) = saved.0 {
            self.carried_tid.insert(name.to_string(), t);
        }
        if let Some(t) = saved.1 {
            self.scalar_types.insert(name.to_string(), t);
        }
    }

    /// Compile one MATCH / OPTIONAL MATCH clause, including its `WHERE`.
    ///
    /// The `WHERE` of an OPTIONAL MATCH is part of the optional match, not a
    /// filter on the result, so it is compiled inside the same scope.
    pub fn compile_match(
        &mut self,
        patterns: &[Pattern],
        optional: bool,
        where_: Option<&Expr>,
    ) -> CResult<()> {
        self.begin_match_clause();
        if optional {
            // An OPTIONAL MATCH that opens the query still yields one row — of
            // nulls when nothing matches. With nothing before it to join onto,
            // its first relation was the plain FROM and its predicates were
            // appended to it as `v_1 n1 AND (…)` — a syntax error. One row to
            // LEFT JOIN onto gives it the shape every later OPTIONAL MATCH has.
            if self.from.is_empty() {
                let a = self.fresh("om");
                self.from.push(format!("(SELECT 1) AS {a}"));
            }
            self.optional = Some(OptionalScope::default());
        }
        let result = (|slf: &mut Self| -> CResult<()> {
            for p in patterns {
                slf.compile_pattern(p, optional)?;
            }
            if let Some(w) = where_ {
                let sql = slf.expr(w, Some("bool"))?;
                slf.constrain(sql);
            }
            Ok(())
        })(self);
        if optional {
            self.close_optional();
        }
        result
    }

    /// `UNWIND expr AS alias` — one row per element of a list.
    ///
    /// Public because the write path needs it too: `UNWIND $rows AS row MATCH …
    /// SET …` is how a Neo4j application writes a batch, and that path builds
    /// its bindings through this compiler rather than re-implementing them.
    pub fn compile_unwind(&mut self, expr: &Expr, alias: &str) -> CResult<()> {
        let src = self.expr(expr, None)?;
        let a = self.fresh("uw");
        let joiner = if self.from.is_empty() { "" } else { "CROSS JOIN " };
        self.from
            .push(format!("{joiner}LATERAL jsonb_array_elements(({src})::jsonb) AS {a}(v)"));
        self.forget_carried(alias);
        self.binds.insert(alias.to_string(), Bind::Scalar { sql: format!("{a}.v") });
        Ok(())
    }

    /// Start from rows a write query has already produced.
    ///
    /// `MERGE (a:A {k: 1}) WITH a MATCH (b:B {k: 2}) MERGE (a)-[:R]->(b)` reads
    /// after it writes. The rows so far travel as the jsonb list `param_key` of
    /// the parameters; each one's variables become bindings here, and a node
    /// whose type every row agrees on (`types`) anchors a pattern on its view.
    pub fn seed_rows(&mut self, param_key: &str, names: &[String], types: &HashMap<String, i32>) {
        let a = self.fresh("rw");
        self.from.push(format!("jsonb_array_elements({PARAM} -> {}) AS {a}(v)", sql_str(param_key)));
        for n in names {
            self.binds.insert(n.clone(), Bind::Scalar { sql: format!("({a}.v -> {})", sql_str(n)) });
            if let Some(t) = types.get(n) {
                self.carried_tid.insert(n.clone(), *t);
            }
        }
    }

    /// Add a predicate, to wherever it belongs: the `ON` of an optional join
    /// while one is open, the query's `WHERE` otherwise.
    fn constrain(&mut self, sql: String) {
        match &mut self.optional {
            Some(scope) => scope.preds.push(sql),
            None => self.wheres.push(sql),
        }
    }

    /// Move one FROM entry to the end, keeping the optional scope's recorded
    /// indices pointing at the same joins.
    fn move_join_to_end(&mut self, idx: usize) {
        let entry = self.from.remove(idx);
        self.from.push(entry);
        let last = self.from.len() - 1;
        if let Some(scope) = &mut self.optional {
            for (_, i) in scope.joins.iter_mut() {
                if *i == idx {
                    *i = last;
                } else if *i > idx {
                    *i -= 1;
                }
            }
        }
    }

    /// Record a join the current optional clause added, so its predicates can
    /// find it later.
    fn note_optional_join(&mut self, alias: &str) {
        let idx = self.from.len() - 1;
        if let Some(scope) = &mut self.optional {
            scope.joins.push((alias.to_string(), idx));
        }
    }

    /// Place each collected predicate on the join that introduced the last
    /// alias it mentions. Joins are emitted in dependency order, so that join
    /// is the earliest point at which the predicate can be evaluated — and the
    /// only one where it filters the optional side without dropping the row.
    fn close_optional(&mut self) {
        let Some(scope) = self.optional.take() else { return };
        for pred in scope.preds {
            let target = scope
                .joins
                .iter()
                .filter(|(alias, _)| mentions_alias(&pred, alias))
                .max_by_key(|(_, idx)| *idx)
                .map(|(_, idx)| *idx);
            match target {
                Some(idx) => {
                    let join = &mut self.from[idx];
                    if let Some(stripped) = join.strip_suffix(" ON true") {
                        *join = format!("{stripped} ON ({pred})");
                    } else {
                        *join = format!("{join} AND ({pred})");
                    }
                }
                // Nothing in this clause is constrained — it is a condition on
                // the outer query, and belongs where any other one does.
                None => self.wheres.push(pred),
            }
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

    /// Can this query tell how many *paths* reach a node, or only which nodes
    /// are reached?
    ///
    /// Cypher's variable-length match yields one row per path, so
    /// `RETURN count(b)` counts walks and `RETURN count(DISTINCT b)` counts
    /// nodes. The first needs trail enumeration; the second does not, and
    /// enumerating trails to answer it costs `degree^k` rows for an answer
    /// bounded by `|V|`.
    ///
    /// The test is deliberately narrow, because being wrong here changes
    /// answers rather than timings:
    ///
    /// * `WITH` disqualifies the query — it can aggregate before the RETURN,
    ///   and this pass does not look inside it.
    /// * `RETURN DISTINCT …` qualifies: duplicate rows cannot survive it.
    /// * Otherwise the projection must aggregate, and every aggregate in it
    ///   must be insensitive to duplicates — `count(DISTINCT x)` is, `count(x)`
    ///   is not, `min`/`max` are whatever their argument is.
    ///
    /// A pattern that binds a path or relationship variable is rejected later,
    /// at the hop itself: those observe multiplicity no matter what RETURN does.
    fn multiplicity_blind(q: &Query) -> bool {
        if q.clauses.iter().any(|c| matches!(c, Clause::With { .. })) {
            return false;
        }
        let Some(Clause::Return(p)) = q.clauses.last() else { return false };
        if p.distinct {
            return true;
        }
        let exprs = || p.items.iter().map(|i| &i.expr).chain(p.order.iter().map(|o| &o.expr));
        exprs().any(|e| e.is_aggregate()) && exprs().all(blind_expr)
    }

    /// Compile a read query, including any `UNION` continuation.
    ///
    /// The parser has always built `Query.union`; nothing read it, so a query
    /// with `UNION` returned its first branch and no error — the worst shape a
    /// bug can take, because the answer looks like an answer.
    ///
    /// Each branch is wrapped in its own subquery rather than concatenated.
    /// `build_select` may emit a leading `WITH`, and `WITH … SELECT … UNION
    /// WITH … SELECT …` is not a thing PostgreSQL parses; a subquery in `FROM`
    /// may carry its own `WITH`, so wrapping is what makes the composition
    /// legal. It also keeps each branch's generated aliases to itself.
    pub fn compile_read(&mut self, q: &Query) -> CResult<Compiled> {
        let head = self.compile_branch(q)?;
        let Some((all, tail)) = &q.union else { return Ok(head) };

        // A fresh compiler per branch: alias counters and bindings are
        // per-branch state, and sharing them across a UNION would leak names
        // from one side into the other.
        let mut c = Compiler::new(&self.graph);
        let rest = c.compile_read(tail)?;
        self.notes.append(&mut c.notes);

        if head.columns != rest.columns {
            return Err(format!(
                "all branches of a UNION must return the same columns in the same order —                  left returns ({}), right returns ({})",
                head.columns.join(", "),
                rest.columns.join(", ")
            ));
        }

        let keyword = if *all { "UNION ALL" } else { "UNION" };
        Ok(Compiled {
            sql: format!(
                "SELECT * FROM (\n{}\n) AS ub_l\n{keyword}\nSELECT * FROM (\n{}\n) AS ub_r",
                head.sql, rest.sql
            ),
            columns: head.columns,
        })
    }

    fn compile_branch(&mut self, q: &Query) -> CResult<Compiled> {
        self.reachability_only = Self::multiplicity_blind(q);
        let mut projection: Option<&Projection> = None;

        for clause in &q.clauses {
            match clause {
                Clause::Match { patterns, optional, where_ } => {
                    self.compile_match(patterns, *optional, where_.as_ref())?
                }
                Clause::Unwind { expr, alias } => self.compile_unwind(expr, alias)?,
                Clause::Call { name, args, yields } => self.compile_call(name, args, yields)?,
                Clause::Return(p) => projection = Some(p),
                Clause::With { proj, where_ } => self.compile_with(proj, where_.as_ref())?,
                _ => return Err("this clause is only valid in a write query".into()),
            }
        }

        // `CALL db.awaitIndexes()` alone is a complete query in Cypher: it
        // returns what the procedure yields.
        if projection.is_none() && matches!(q.clauses.last(), Some(Clause::Call { .. })) {
            let items = self
                .last_yields
                .iter()
                .map(|n| ReturnItem { expr: Expr::Var(n.clone()), alias: Some(n.clone()) })
                .collect();
            return self.build_select(&Projection { items, ..Default::default() });
        }
        let proj = projection.ok_or("a read query must end with RETURN")?;
        self.build_select(proj)
    }

    /// `WITH …` — the clause that makes Cypher a pipeline.
    ///
    /// Everything compiled so far becomes a subquery; its projected names
    /// become the only bindings visible afterwards. That is exactly Cypher's
    /// rule ("WITH is the horizon"), and it falls out of the implementation
    /// rather than being enforced separately: nothing but the projected columns
    /// survives into the next segment.
    ///
    /// Aggregation works for free — `WITH a, count(*) AS n` is a grouped SELECT,
    /// and the `WHERE` after it filters the grouped rows, which is `HAVING`.
    pub(crate) fn compile_with(&mut self, proj: &Projection, where_: Option<&Expr>) -> CResult<()> {
        // What each projected name is, decided while the names it is computed
        // from are still bound.
        let mut carried_tid: HashMap<String, i32> = HashMap::new();
        let mut scalar_types: HashMap<String, String> = HashMap::new();
        let mut carried_lists = std::collections::HashSet::new();
        for item in &proj.items {
            if matches!(item.expr, Expr::Star) {
                carried_lists.extend(self.carried_lists.iter().cloned());
                for (v, b) in &self.binds {
                    if let Bind::Node { tid: Some(t), .. } = b {
                        carried_tid.insert(v.clone(), *t);
                    }
                }
                carried_tid.extend(self.carried_tid.iter().map(|(k, v)| (k.clone(), *v)));
                scalar_types.extend(self.scalar_types.iter().map(|(k, v)| (k.clone(), v.clone())));
                continue;
            }
            let name = item.alias.clone().unwrap_or_else(|| item.expr.default_alias());
            if self.is_list_value(&item.expr) {
                carried_lists.insert(name.clone());
            }
            if let Expr::Var(v) = &item.expr {
                match self.binds.get(v) {
                    Some(Bind::Node { tid: Some(t), .. }) => {
                        carried_tid.insert(name.clone(), *t);
                    }
                    Some(Bind::Scalar { .. }) => {
                        if let Some(t) = self.carried_tid.get(v) {
                            carried_tid.insert(name.clone(), *t);
                        }
                    }
                    _ => {}
                }
            }
            if let Some(ty) = self.type_of(&item.expr) {
                if is_scalar_type(&ty) {
                    scalar_types.insert(name, ty);
                }
            }
        }
        let inner = self.build_tabular(proj)?;
        let alias = self.fresh("w");

        // Names the segment exported, in the order the SELECT produced them.
        let cols: Vec<String> = inner.columns.clone();
        let quoted: Vec<String> = cols.iter().map(|c| quote_ident(c)).collect();

        self.from = vec![format!("({}) AS {alias}({})", inner.sql, quoted.join(", "))];
        self.wheres.clear();
        self.rel_ids.clear();
        self.binds = cols
            .iter()
            .zip(&quoted)
            .map(|(name, q)| {
                (name.clone(), Bind::Scalar { sql: format!("{alias}.{q}") })
            })
            .collect();
        self.carried_tid = carried_tid;
        self.scalar_types = scalar_types;
        self.carried_lists = carried_lists;

        // `WITH … WHERE` filters what the horizon produced, so it is compiled
        // against the new bindings, not the old ones.
        if let Some(w) = where_ {
            let sql = self.expr(w, Some("bool"))?;
            self.wheres.push(sql);
        }
        Ok(())
    }

    /// `CALL proc(args) YIELD a, b AS c`.
    ///
    /// The procedure becomes a relation in the FROM list and its yielded
    /// columns become ordinary bindings, so everything after it — WHERE,
    /// another MATCH, RETURN — treats them like any other bound value.
    pub(crate) fn compile_call(
        &mut self,
        name: &str,
        args: &[Expr],
        yields: &[(String, String)],
    ) -> CResult<()> {
        use crate::compat::procs;

        let mut planned = Vec::new();
        for a in args {
            planned.push(match a {
                Expr::Lit(Lit::Str(s)) => procs::Arg::Str(s.clone()),
                // A procedure that walks from a node wants the node's id, not
                // its jsonb — pass the column so the walk stays a join.
                Expr::Var(v) => match self.binds.get(v) {
                    Some(Bind::Node { alias, .. }) => procs::Arg::NodeId(format!("{alias}.id")),
                    _ => procs::Arg::Sql(self.expr(a, None)?),
                },
                _ => procs::Arg::Sql(self.expr(a, None)?),
            });
        }

        let alias = self.fresh("cp");
        let plan = procs::plan(&self.graph, self.gid, name, &planned, &alias)?;

        let joiner = if self.from.is_empty() {
            String::new()
        } else if plan.lateral {
            "CROSS JOIN LATERAL ".into()
        } else {
            "CROSS JOIN ".into()
        };
        self.from.push(format!("{joiner}{}", plan.from));

        // With no YIELD every column comes into scope under its own name, which
        // is what Cypher does for a standalone CALL.
        let wanted: Vec<(String, String)> = if yields.is_empty() {
            plan.columns.iter().map(|c| (c.name.to_string(), c.name.to_string())).collect()
        } else {
            yields.to_vec()
        };
        self.last_yields = wanted.iter().map(|(_, a)| a.clone()).collect();
        for (col, as_name) in wanted {
            let found = plan
                .columns
                .iter()
                .find(|c| c.name.eq_ignore_ascii_case(&col))
                .ok_or_else(|| {
                    format!(
                        "procedure '{name}' does not yield '{col}'. it yields: {}",
                        plan.columns.iter().map(|c| c.name).collect::<Vec<_>>().join(", ")
                    )
                })?;
            self.forget_carried(&as_name);
            self.binds.insert(as_name, Bind::Scalar { sql: found.sql.clone() });
        }
        Ok(())
    }

    /// The parts of a projection, before deciding how to shape the result.
    ///
    /// `RETURN` packs them into one jsonb object per row; `WITH` keeps them as
    /// columns so the next segment can join against them. Both need the same
    /// SELECT underneath, so it is built once here.
    fn build_core(&mut self, proj: &Projection) -> CResult<(String, String, String, Vec<String>)> {
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
            let sql = self.expr_for_output(&item.expr)?;
            let name = item.alias.clone().unwrap_or_else(|| item.expr.default_alias());
            if has_agg && !item.expr.is_aggregate() {
                group_by.push(sql.clone());
            } else if has_agg {
                // `a + collect(m.key)`: the `a` outside the aggregate is a
                // grouping key, as in Cypher — SQL wants it said.
                for r in bare_references(&item.expr) {
                    let g = self.expr(&r, None)?;
                    if !group_by.contains(&g) {
                        group_by.push(g);
                    }
                }
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

        // Which projection aliases name an aggregate. `ORDER BY <alias>` where
        // the alias is `count(*) AS count` must not push that alias into
        // GROUP BY — grouping by an aggregate is not a thing, and PostgreSQL
        // says so. The projection has already decided each alias's role.
        let alias_is_agg: std::collections::HashMap<String, bool> = proj
            .items
            .iter()
            .map(|item| {
                let name = item.alias.clone().unwrap_or_else(|| item.expr.default_alias());
                (name, item.expr.is_aggregate())
            })
            .collect();

        let mut order_cols = Vec::new();
        let mut order_by = Vec::new();
        for (i, o) in proj.order.iter().enumerate() {
            let mut names_an_aggregate = false;
            let sql = match &o.expr {
                Expr::Var(v) if !self.binds.contains_key(v) => {
                    names_an_aggregate = alias_is_agg.get(v).copied().unwrap_or(false);
                    alias_sql
                        .get(v)
                        .cloned()
                        .ok_or_else(|| format!("ORDER BY refers to unknown name '{v}'"))?
                }
                // `RETURN x {.name, .age} AS a ORDER BY a.age`: a key of a
                // projected value, read from the projection.
                Expr::Prop(base, p)
                    if matches!(&**base, Expr::Var(v) if !self.binds.contains_key(v) && alias_sql.contains_key(v)) =>
                {
                    let Expr::Var(v) = &**base else { unreachable!() };
                    names_an_aggregate = alias_is_agg.get(v).copied().unwrap_or(false);
                    format!("(to_jsonb({}) -> {})", alias_sql[v], sql_str(p))
                }
                _ => self.expr(&o.expr, None)?,
            };
            if has_agg && !o.expr.is_aggregate() && !names_an_aggregate && !group_by.contains(&sql) {
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

        Ok((with, inner, format!("{order}{limit}{skip}"), names))
    }

    fn build_select(&mut self, proj: &Projection) -> CResult<Compiled> {
        let (with, inner, tail, names) = self.build_core(proj)?;
        let json_pairs: Vec<String> = names
            .iter()
            .enumerate()
            .map(|(i, n)| format!("{}, t.c{i}", sql_str(n)))
            .collect();
        Ok(Compiled {
            sql: format!(
                "{with}SELECT {} AS row FROM (\n{inner}\n) t{tail}",
                views::json_object(&json_pairs)
            ),
            columns: names,
        })
    }

    /// The same projection, left as columns instead of packed into one jsonb
    /// object. `WITH` needs it: the next segment joins against these columns,
    /// and it cannot join against a blob.
    fn build_tabular(&mut self, proj: &Projection) -> CResult<Compiled> {
        let (with, inner, tail, names) = self.build_core(proj)?;
        // Every column crosses the horizon as jsonb, whatever it was before.
        // The alternative — keeping each column's SQL type — would mean the
        // next segment had to know which of its bindings is a node (jsonb) and
        // which is a count (bigint) before it could compile a comparison. One
        // representation for all of them is what makes the binding after a WITH
        // the same kind of thing as any other scalar binding.
        let projected: Vec<String> = names
            .iter()
            .enumerate()
            .map(|(i, n)| format!("to_jsonb(t.c{i}) AS {}", quote_ident(n)))
            .collect();
        Ok(Compiled {
            sql: format!(
                "{with}SELECT {} FROM (\n{inner}\n) t{tail}",
                projected.join(", ")
            ),
            columns: names,
        })
    }

    // ----------------------------------------------------------------------
    // Patterns
    // ----------------------------------------------------------------------

    pub fn compile_pattern(&mut self, p: &Pattern, optional: bool) -> CResult<()> {
        // An OPTIONAL pattern has to start at a node that is already bound:
        // each new node must join *after* the hop that reaches it, or its join
        // stays `ON true` and the optional side multiplies against everything
        // (`OPTIONAL MATCH (s:Shipment)-[:SHIPS]->(po)` with `po` bound gave
        // every shipment). Starting from the bound node, the left part is
        // walked backwards and the rest as written.
        if optional && p.path_var.is_none() {
            if let Some(k) = self.first_bound_node(p) {
                if k > 0 {
                    let left = Pattern { path_var: None, elems: reverse_elems(&p.elems[..=k]) };
                    self.compile_pattern(&left, optional)?;
                    if k + 1 < p.elems.len() {
                        let right = Pattern { path_var: None, elems: p.elems[k..].to_vec() };
                        self.compile_pattern(&right, optional)?;
                    }
                    return Ok(());
                }
            }
        }
        if optional {
            self.notes.push("OPTIONAL MATCH compiled as LEFT JOIN LATERAL".into());
        }
        let mut prev_node: Option<String> = None;
        let mut pending_rel: Option<&RelPat> = None;
        // For a path variable: the relationships of each hop and the nodes
        // reached by it, as jsonb arrays concatenated in order.
        let mut path_rels: Vec<String> = Vec::new();
        let mut path_nodes: Vec<String> = Vec::new();

        // `MATCH p = (a)-[*1..3]->(b)` binds the paths themselves, so this
        // pattern must enumerate them however multiplicity-blind the RETURN is.
        let outer_reachability = self.reachability_only;
        if p.path_var.is_some() {
            self.reachability_only = false;
        }

        for (idx, elem) in p.elems.iter().enumerate() {
            match elem {
                PatElem::Node(np) => {
                    let mark = self.from.len();
                    let inferred = self.infer_virtual_label(p, idx, np, prev_node.as_deref());
                    let np = inferred.as_ref().unwrap_or(np);
                    let alias = self.bind_node(np, optional)?;
                    let node_join_added = self.from.len() > mark;
                    if let (Some(prev), Some(rel)) = (prev_node.clone(), pending_rel.take()) {
                        self.last_var_hop = None;
                        let hop = self.join_rel(&prev, rel, &alias, optional)?;
                        if p.path_var.is_some() {
                            path_rels.push(hop);
                            path_nodes.push(match self.last_var_hop.take() {
                                Some(w) => var_hop_nodes(&prev, &w),
                                None => format!("jsonb_build_array({})", self.alias_json(&alias)),
                            });
                        }
                        // The node is constrained by the hop that reaches it, so
                        // its join has to come *after* the hop's — otherwise the
                        // predicate cannot be an ON condition and the node joins
                        // to everything. Only matters under OPTIONAL MATCH,
                        // where predicates live in ON rather than WHERE.
                        if node_join_added && mark > 0 && self.from.len() > mark + 1 {
                            self.move_join_to_end(mark);
                        }
                    }
                    if p.path_var.is_some() && prev_node.is_none() {
                        path_nodes.push(format!("jsonb_build_array({})", self.alias_json(&alias)));
                    }
                    prev_node = Some(alias);
                }
                PatElem::Rel(rp) => pending_rel = Some(rp),
            }
        }

        if let (Some(pv), Some(last)) = (&p.path_var, &prev_node) {
            let rels = if path_rels.is_empty() { "'[]'::jsonb".to_string() } else { path_rels.join(" || ") };
            let expr = format!(
                "(CASE WHEN {last}.id IS NULL THEN NULL ELSE jsonb_build_object('_path', true, \
                 'nodes', {}, 'relationships', {rels}) END)",
                path_nodes.join(" || ")
            );
            self.binds.insert(pv.clone(), Bind::Path { hops_expr: expr });
        }
        self.reachability_only = outer_reachability;
        Ok(())
    }

    /// Index of the first node in the pattern whose variable is already bound.
    fn first_bound_node(&self, p: &Pattern) -> Option<usize> {
        p.elems.iter().position(|e| {
            matches!(e, PatElem::Node(NodePat { var: Some(v), .. })
                if matches!(self.binds.get(v), Some(Bind::Node { .. })))
        })
    }

    /// Resolve a label set, reporting separately whether it can match at all.
    /// Callers decide what "cannot match" means where they stand: an empty
    /// result for a plain MATCH, a row of NULLs under OPTIONAL MATCH.
    fn resolve_label_match(&mut self, labels: &[String]) -> CResult<(Option<i32>, bool)> {
        match types::resolve_label_set(self.gid, &self.graph, labels)? {
            types::LabelMatch::Any => Ok((None, true)),
            types::LabelMatch::Type(t) => Ok((Some(t), true)),
            types::LabelMatch::Nothing => match self.virtual_under_umbrella(labels) {
                Some(t) => Ok((Some(t), true)),
                None => Ok((None, false)),
            },
        }
    }

    /// `(:_Entity:Customer)` where `Customer` is a virtual type.
    ///
    /// A label set means "the most specific of these, which is a subtype of the
    /// rest". A virtual type usually is not under the umbrella supertype an
    /// application puts every class beneath (`_Entity` in Ontology Studio):
    /// its rows live in the source, so the marker write that places a stored
    /// type there cannot be made for it, and a type mapped before it was
    /// published starts as a root. The set then resolved to nothing — an empty
    /// MATCH, and over a virtual relationship an error about unlabelled ends —
    /// although the application meant exactly the virtual type.
    ///
    /// So beside exactly one virtual type, a label that names a *supertype* —
    /// a type others descend from, or an abstract one — is read as that
    /// umbrella and set aside, with a note. A label that names nothing, or a
    /// concrete type with no subtypes, still empties the match.
    fn virtual_under_umbrella(&mut self, labels: &[String]) -> Option<i32> {
        if labels.len() < 2 {
            return None;
        }
        let ids: Vec<i32> =
            labels.iter().map(|l| types::try_type_id(self.gid, l)).collect::<Option<_>>()?;
        let virtuals: Vec<i32> = ids
            .iter()
            .copied()
            .filter(|t| crate::fabric::virtual_source(*t).is_some())
            .collect();
        let [v] = virtuals.as_slice() else { return None };
        let mut set_aside = Vec::new();
        for (name, &t) in labels.iter().zip(&ids) {
            if t == *v || crate::catalog::labeling::og_is_subtype(*v, t) {
                continue;
            }
            if !is_umbrella(t) {
                return None;
            }
            set_aside.push(name.as_str());
        }
        let vname = types::try_type_name(*v).unwrap_or_default();
        self.notes.push(format!(
            "label(s) :{} set aside — virtual type {vname} is not placed under them",
            set_aside.join(":")
        ));
        Some(*v)
    }

    fn resolve_label(&mut self, labels: &[String]) -> CResult<Option<i32>> {
        let (tid, matchable) = self.resolve_label_match(labels)?;
        if !matchable {
            self.constrain("false".into());
        }
        Ok(tid)
    }

    fn bind_node(&mut self, np: &NodePat, optional: bool) -> CResult<String> {
        // Nothing can be LEFT JOINed onto an empty FROM list; the first
        // relation in the query is always plain.
        let optional = optional && !self.from.is_empty();
        // A variable that arrived as jsonb — yielded by a procedure, or carried
        // through an UNWIND — can still anchor a pattern. Join the node view on
        // its identifier and it becomes an ordinary node binding from here on.
        if let Some(v) = &np.var {
            if let Some(Bind::Scalar { sql }) = self.binds.get(v).cloned() {
                if let Some(alias) = self.bind_carried_node(v, &sql, np)? {
                    return Ok(alias);
                }
                let alias = self.fresh("n");
                let joiner = if self.from.is_empty() {
                    ""
                } else if self.optional.is_some() {
                    "LEFT JOIN "
                } else {
                    "CROSS JOIN "
                };
                let on = if self.optional.is_some() { " ON true" } else { "" };
                self.from.push(format!("{joiner}og_data.og_node {alias}{on}"));
                self.note_optional_join(&alias);
                self.constrain(format!("{alias}.id = (({sql}) ->> '_id')::int8"));
                self.guard_untyped_nodes();
                self.binds.insert(v.clone(), Bind::Node { alias: alias.clone(), tid: None });
                if !np.labels.is_empty() {
                    let (want, matchable) = self.resolve_label_match(&np.labels)?;
                    if !matchable {
                        self.constrain("false".into());
                    } else if let Some(w) = want {
                        self.constrain(format!(
                            "og_is_subtype({alias}.type_id, {w})"
                        ));
                    }
                }
                self.push_prop_filters(&alias, None, &np.props)?;
                return Ok(alias);
            }
        }

        // Re-using an existing variable: reuse its alias, do not join again.
        if let Some(v) = &np.var {
            if let Some(Bind::Node { alias, tid }) = self.binds.get(v).cloned() {
                if !np.labels.is_empty() {
                    let want = self.resolve_label(&np.labels)?;
                    if let (Some(w), Some(t)) = (want, tid) {
                        if w != t && !crate::catalog::labeling::og_is_subtype(t, w) {
                            self.constrain("false".into());
                        }
                    }
                }
                self.push_prop_filters(&alias, tid, &np.props)?;
                return Ok(alias);
            }
        }

        let (tid, matchable) = self.resolve_label_match(&np.labels)?;
        let alias = self.fresh("n");
        let rel = match tid {
            Some(t) => views::ensure_view(t, false),
            None => {
                self.guard_untyped_nodes();
                self.graph_nodes()
            }
        };

        // An unmatchable label empties this binding only. Under OPTIONAL MATCH
        // that means NULLs on the join, not a query that returns nothing.
        let on = if matchable { "true" } else { "false" };
        let join = if self.from.is_empty() {
            if !matchable {
                self.constrain("false".into());
            }
            format!("{rel} {alias}")
        } else if optional {
            format!("LEFT JOIN {rel} {alias} ON {on}")
        } else {
            if !matchable {
                self.constrain("false".into());
            }
            format!("CROSS JOIN {rel} {alias}")
        };
        self.from.push(join);
        self.note_optional_join(&alias);

        self.alias_tid.insert(alias.clone(), tid);
        if !matchable {
            self.unmatchable.insert(alias.clone());
        }
        if let Some(v) = &np.var {
            self.binds.insert(v.clone(), Bind::Node { alias: alias.clone(), tid });
        }
        self.push_prop_filters(&alias, tid, &np.props)?;
        Ok(alias)
    }

    /// A node carried through `WITH` (or UNWIND, or a procedure) as jsonb,
    /// anchoring a pattern again, when what type it is can be known — from
    /// the pattern's labels, or from what the `WITH` carried.
    ///
    /// It joins that type's view rather than the node registry. The registry
    /// holds only stored nodes, so a virtual node — whose rows live in the
    /// source — found nothing there, and with no type the hop over a virtual
    /// relationship had no columns to join on ("needs both ends labelled").
    /// `None` leaves the registry path to the caller.
    fn bind_carried_node(&mut self, v: &str, sql: &str, np: &NodePat) -> CResult<Option<String>> {
        let (tid, matchable) = if np.labels.is_empty() {
            (self.carried_tid.get(v).copied(), true)
        } else {
            self.resolve_label_match(&np.labels)?
        };
        let (Some(t), true) = (tid, matchable) else { return Ok(None) };
        let alias = self.fresh("n");
        let rel = views::ensure_view(t, false);
        let (joiner, on) = if self.from.is_empty() {
            ("", "")
        } else if self.optional.is_some() {
            ("LEFT JOIN ", " ON true")
        } else {
            ("CROSS JOIN ", "")
        };
        self.from.push(format!("{joiner}{rel} {alias}{on}"));
        self.note_optional_join(&alias);
        self.constrain(format!("{alias}.id = (({sql}) ->> '_id')::int8"));
        // A virtual node is also matched on its key, which a source can be
        // sent; the derived id is computed here and cannot.
        if let Some(key) = virtual_key_property(t).filter(|k| k != "_id") {
            let (col, ty) = self.prop_sql(&alias, Some(t), &key);
            if let Some(ty) = ty.filter(|ty| is_scalar_type(ty)) {
                self.constrain(format!("{col} = (({sql}) ->> {})::{ty}", sql_str(&key)));
            }
        }
        self.alias_tid.insert(alias.clone(), Some(t));
        self.forget_carried(v);
        self.binds.insert(v.to_string(), Bind::Node { alias: alias.clone(), tid: Some(t) });
        self.push_prop_filters(&alias, Some(t), &np.props)?;
        Ok(Some(alias))
    }

    /// The single relationship type of a hop, when it is a virtual one.
    fn virtual_rel_of(&self, rel: &RelPat) -> Option<(i32, crate::fabric::VirtualRel)> {
        if rel.types.len() != 1 {
            return None;
        }
        let rtid = types::try_type_id(self.gid, &rel.types[0])?;
        crate::fabric::virtual_rel(rtid).map(|v| (rtid, v))
    }

    /// `MATCH (s:Supplier)<-[:ORDERED_FROM]-(po)` — the unlabelled end of a
    /// virtual hop has only one type it can be, and without that type there are
    /// no columns to join on. Stored adjacency does not need this; a join does.
    fn infer_virtual_label(
        &mut self,
        p: &Pattern,
        idx: usize,
        np: &NodePat,
        prev_alias: Option<&str>,
    ) -> Option<NodePat> {
        if !np.labels.is_empty() {
            return None;
        }
        if let Some(v) = &np.var {
            match self.binds.get(v) {
                None => {}
                // A value carried through WITH whose type was not carried: the
                // hop decides, as for a fresh variable.
                Some(Bind::Scalar { .. }) if !self.carried_tid.contains_key(v) => {}
                Some(_) => return None,
            }
        }
        // As the far end of the previous hop, then as the near end of the next.
        let sides = [
            (idx.checked_sub(1).and_then(|i| p.elems.get(i)), true),
            (p.elems.get(idx + 1), false),
        ];
        for (elem, is_far_end) in sides {
            let Some(PatElem::Rel(rel)) = elem else { continue };
            let Some((_, v)) = self.virtual_rel_of(rel) else { continue };
            let tid = match (&rel.dir, is_far_end) {
                (Dir::Out, true) | (Dir::In, false) => v.dst_type,
                (Dir::In, true) | (Dir::Out, false) => v.src_type,
                // Undirected: the other end's type says which end this is.
                (Dir::Both, _) => {
                    let other = if is_far_end {
                        prev_alias.and_then(|a| self.alias_tid.get(a).copied().flatten())
                    } else {
                        match p.elems.get(idx + 2) {
                            Some(PatElem::Node(o)) => self.node_pat_type(o),
                            _ => None,
                        }
                    };
                    let Some(o) = other else { continue };
                    let is = |t: i32, want: i32| {
                        t == want || crate::catalog::labeling::og_is_subtype(t, want)
                    };
                    match (is(o, v.src_type), is(o, v.dst_type)) {
                        (true, false) => v.dst_type,
                        (false, true) => v.src_type,
                        (true, true) if v.src_type == v.dst_type => v.src_type,
                        _ => continue,
                    }
                }
            };
            let name = types::try_type_name(tid)?;
            let mut out = np.clone();
            out.labels = vec![name];
            return Some(out);
        }
        None
    }

    /// The type a node pattern not yet bound stands for, without binding it:
    /// its labels, or the type its variable is already bound to.
    fn node_pat_type(&mut self, np: &NodePat) -> Option<i32> {
        if np.labels.is_empty() {
            return match np.var.as_ref().and_then(|v| self.binds.get(v)) {
                Some(Bind::Node { tid, .. }) => *tid,
                Some(Bind::Scalar { .. }) => np.var.as_ref().and_then(|v| self.carried_tid.get(v).copied()),
                _ => None,
            };
        }
        // Unknown labels are reported once, when the node itself is bound.
        if np.labels.iter().any(|l| types::try_type_id(self.gid, l).is_none()) {
            return None;
        }
        let notes = self.notes.len();
        let got = self.resolve_label_match(&np.labels).ok().and_then(|(t, ok)| t.filter(|_| ok));
        self.notes.truncate(notes);
        got
    }

    /// A hop over a virtual relationship is a join on the declared keys: no
    /// adjacency is read, so neither end has to be stored here. With both ends
    /// on the same foreign server the planner can send the join itself there.
    fn join_virtual_rel(
        &mut self,
        from_alias: &str,
        rel: &RelPat,
        to_alias: &str,
        rtid: i32,
        v: crate::fabric::VirtualRel,
    ) -> CResult<String> {
        let rname = &rel.types[0];
        if rel.range.is_some() {
            return Err(format!(
                "virtual relationship '{rname}' cannot be traversed with a variable length yet"
            ));
        }
        if !rel.props.is_empty() {
            return Err(format!("virtual relationship '{rname}' has no properties to match"));
        }
        let ft = self.alias_tid.get(from_alias).copied().flatten();
        let tt = self.alias_tid.get(to_alias).copied().flatten();
        let is = |t: Option<i32>, want: i32| {
            t.map_or(false, |t| t == want || crate::catalog::labeling::og_is_subtype(t, want))
        };
        let forward = match rel.dir {
            Dir::Out => true,
            Dir::In => false,
            Dir::Both => is(ft, v.src_type) && is(tt, v.dst_type),
        };
        let (s_alias, s_tid, d_alias, d_tid) = if forward {
            (from_alias, ft, to_alias, tt)
        } else {
            (to_alias, tt, from_alias, ft)
        };
        if s_tid.is_none() || d_tid.is_none() {
            // An end whose labels can match nothing (`(:Unrelated:Other)`) has
            // already emptied the match; the hop then has nothing to join and
            // is not the error worth reporting.
            let unmatchable = |a: &str| self.unmatchable.contains(a);
            if (s_tid.is_none() && unmatchable(s_alias)) || (d_tid.is_none() && unmatchable(d_alias)) {
                self.constrain(never(s_alias, d_alias));
                return Ok("NULL::jsonb".into());
            }
            return Err(format!(
                "virtual relationship '{rname}' needs both ends labelled — it is a join on their keys. \
                 Label one end, or give the hop a direction, and the other end is known from it: \
                 (a:Label)-[:{rname}]-(b) or (a)-[:{rname}]->(b)"
            ));
        }
        if !is(s_tid, v.src_type) || !is(d_tid, v.dst_type) {
            // Wrong endpoint types: the relationship cannot exist between them.
            // Said about the hop's ends, so that under OPTIONAL MATCH it lands
            // on the optional join (NULLs) rather than emptying the whole query.
            // No join on the keys: those types need not have the columns, and
            // comparing what they do have failed (`text = integer`) where
            // Neo4j simply matches nothing.
            self.constrain(never(s_alias, d_alias));
        } else {
            let (lhs, _) = self.prop_sql(s_alias, s_tid, &v.src_prop);
            let (rhs, _) = self.prop_sql(d_alias, d_tid, &v.dst_prop);
            self.constrain(format!("{lhs} = {rhs}"));
            self.notes.push(format!(
                "'{rname}' is virtual: compiled as the join {lhs} = {rhs}, no adjacency read"
            ));
        }

        let rel_json = format!(
            "jsonb_build_object('_id', og_make_id(0, {rtid}, \
             hashtextextended({s_alias}.id::text || ':' || {d_alias}.id::text, 0) & {mask}), \
             '_type', {tn}, '_src', {s_alias}.id, '_dst', {d_alias}.id)",
            mask = (1_i64 << 36) - 1,
            tn = sql_str(rname),
        );
        if let Some(var) = &rel.var {
            self.binds.insert(var.clone(), Bind::Scalar { sql: rel_json.clone() });
        }
        Ok(format!("jsonb_build_array({rel_json})"))
    }

    fn push_prop_filters(
        &mut self,
        alias: &str,
        tid: Option<i32>,
        props: &[(String, Expr)],
    ) -> CResult<()> {
        for (k, v) in props {
            let (lhs, ty) = self.prop_sql(alias, tid, k);
            // A property with no declared type (an unlabelled node, a key only
            // the extension payload holds) is compared as JSON: read as text it
            // met a jsonb value (`UNWIND $ids AS x MATCH (n {k: x})`) or a
            // number as `text = jsonb` / `text = integer`, which do not exist.
            if ty.is_none() {
                let lj = self.prop_sql_json(alias, tid, k);
                let rj = self.jsonb_of(v)?;
                self.constrain(format!("{lj} = {rj}"));
                continue;
            }
            let rhs = self.expr(v, ty.as_deref())?;
            self.constrain(format!("{lhs} = {rhs}"));
        }
        Ok(())
    }

    /// The node registry, narrowed to this query's graph.
    ///
    /// `og_node` is shared by every graph in the database — a node's graph is
    /// its type's — so an unlabelled `MATCH (n)` or `(a)-[r]->(b)` read the
    /// nodes of every graph (project), and `MATCH (n) DETACH DELETE n` emptied
    /// all of them. A labelled match was always confined by its label. The
    /// graph's types are looked up when the query runs, not when it is
    /// compiled: a compiled plan is cached, and a type another session creates
    /// meanwhile must not be missed.
    fn graph_nodes(&self) -> String {
        format!(
            "(SELECT gn.* FROM og_data.og_node gn WHERE gn.type_id IN \
             (SELECT gt.type_id FROM og_catalog.type gt WHERE gt.graph_id = {}))",
            self.gid
        )
    }

    /// An unlabelled node pattern scans the node registry, which holds only
    /// what exists now. In a graph with temporal node types that cannot answer
    /// a point-in-time query, so it refuses to (spec 013 FR-010).
    fn guard_untyped_nodes(&mut self) {
        if !crate::temporal::temporal_tables(self.gid, 'n').is_empty() {
            self.constrain("og_require_current('an unlabelled node pattern')".into());
        }
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
        if let Some((rtid, v)) = self.virtual_rel_of(rel) {
            return self.join_virtual_rel(from_alias, rel, to_alias, rtid, v);
        }
        let etypes = if rel.types.is_empty() {
            None
        } else {
            // A relationship type nobody has written yet simply contributes no
            // ids; the hop then finds no neighbours. That keeps an inner MATCH
            // empty and leaves an OPTIONAL MATCH with its NULLs, which is what
            // Cypher does — and is why this must not push a global `false`.
            let mut ids = Vec::new();
            for t in &rel.types {
                if let Some(tid) = types::try_type_id(self.gid, t) {
                    ids.extend(crate::catalog::labeling::og_subtypes(tid));
                }
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

        // Temporal relation types this hop may cross. Their adjacency segments
        // know only the present, so a hop reads them from the edge tables —
        // current and history, filtered to the session's instant (spec 013).
        let temporal_rels: Vec<i32> = match &etypes {
            Some(ids) => ids
                .iter()
                .copied()
                .filter(|t| {
                    types::storage_table(*t).is_some_and(|s| s.starts_with("og_data.e_"))
                        && crate::temporal::is_temporal(*t)
                })
                .collect(),
            None => crate::temporal::temporal_tables(self.gid, 'e'),
        };

        if let Some((min, max)) = rel.range {
            if !temporal_rels.is_empty() {
                self.constrain(
                    "og_require_current('variable-length traversal over a temporal relationship type')"
                        .into(),
                );
            }
            let w = self.fresh("vl");
            let joiner = if optional { "LEFT JOIN LATERAL" } else { "CROSS JOIN LATERAL" };
            let on = if optional { " ON true" } else { "" };
            // `og_vlp` enumerates trails, so it produces degree^k rows and can
            // bind a path. When nobody can observe the multiplicity — no path
            // or relationship variable, and a projection that collapses
            // duplicates — the same answer comes out of a BFS that visits each
            // node once, and the cost stops being exponential in the depth.
            // `min > 1` is where the two functions stop meaning the same thing.
            // `og_vlp` enumerates trails, so it answers "is there a walk whose
            // length falls in [min, max]". `og_reach` visits each node once and
            // emits it at its *shortest* distance, so a node one hop away that
            // is also reachable in three is marked visited at depth 1, never
            // emitted, and never reconsidered — the rewrite silently drops it.
            // The regression suite only ever writes `*1..k`, which is why this
            // has never been observed. Reachability is an optimisation, and an
            // optimisation that changes the answer is not one.
            let f = if rel.var.is_none()
                && min <= 1
                && self.reachability_only
                && prefer_reachability(max)
            {
                self.notes.push(format!(
                    "variable-length hop compiled as reachability (og_reach): \
                     no path is observable, so trails are not enumerated"
                ));
                "og_reach"
            } else {
                "og_vlp"
            };
            self.from.push(format!(
                "{joiner} {f}({from_alias}.id, {etype_pred}, {dir_lit}::\"char\", {min}, {max}) {w}{on}"
            ));
            self.note_optional_join(&w);
            self.constrain(format!("{to_alias}.id = {w}.node"));
            // A relationship variable on a variable-length hop is the list of
            // relationships walked, as in Cypher (it used to be their ids).
            let rels = var_hop_rels(&w);
            if let Some(v) = &rel.var {
                self.forget_carried(v);
                self.binds.insert(v.clone(), Bind::Scalar { sql: rels.clone() });
                self.carried_lists.insert(v.clone());
            }
            self.last_var_hop = Some(w);
            return Ok(rels);
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
        if temporal_rels.is_empty() {
            self.from.push(format!(
                "{joiner} (SELECT u.nbr, u.eid FROM og_data.og_adj {a}, \
                 LATERAL unnest({a}.nbr, {a}.eid) AS u(nbr, eid) \
                 WHERE {a}.src = {from_alias}.id AND {dir_pred}{type_pred}) {u}{on}"
            ));
        } else {
            let tlist = temporal_rels.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(",");
            let mut parts = vec![format!(
                "SELECT u.nbr, u.eid FROM og_data.og_adj {a}, \
                 LATERAL unnest({a}.nbr, {a}.eid) AS u(nbr, eid) \
                 WHERE {a}.src = {from_alias}.id AND {dir_pred}{type_pred} \
                   AND {a}.etype <> ALL(ARRAY[{tlist}]::int4[])"
            )];
            for t in &temporal_rels {
                let Some(erel) = crate::temporal::edge_relation(*t) else { continue };
                let te = self.fresh("te");
                if matches!(rel.dir, Dir::Out | Dir::Both) {
                    parts.push(format!(
                        "SELECT {te}.dst, {te}.id FROM {erel} {te} WHERE {te}.src = {from_alias}.id"
                    ));
                }
                if matches!(rel.dir, Dir::In | Dir::Both) {
                    let te2 = self.fresh("te");
                    parts.push(format!(
                        "SELECT {te2}.src, {te2}.id FROM {erel} {te2} WHERE {te2}.dst = {from_alias}.id"
                    ));
                }
            }
            self.from.push(format!("{joiner} ({}) {u}{on}", parts.join(" UNION ALL ")));
            // A relationship variable without one known type is rendered from
            // the edge registry, which has no history.
            if rel.var.is_some() && rel.types.len() != 1 {
                self.constrain(
                    "og_require_current('a relationship variable without a single type')".into(),
                );
            }
        }
        self.note_optional_join(&u);
        self.constrain(format!("{to_alias}.id = {u}.nbr"));

        if !optional {
            for other in std::mem::take(&mut self.rel_ids) {
                self.constrain(format!("{u}.eid <> {other}"));
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
                    let ejoin = if optional { "LEFT JOIN" } else { "JOIN" };
                    self.from.push(format!("{ejoin} {ev} {ea} ON {ea}.id = {u}.eid"));
                    self.note_optional_join(&ea);
                    self.binds.insert(
                        v.clone(),
                        Bind::Rel { alias: Some(ea.clone()), tid: Some(t), id_expr: format!("{u}.eid") },
                    );
                    self.push_rel_prop_filters(&ea, Some(t), &rel.props)?;
                    return Ok(format!("jsonb_build_array({})", self.rel_json(&ea, t)));
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
                let ejoin = if optional { "LEFT JOIN" } else { "JOIN" };
                    self.from.push(format!("{ejoin} {ev} {ea} ON {ea}.id = {u}.eid"));
                    self.note_optional_join(&ea);
                self.push_rel_prop_filters(&ea, Some(t), &rel.props)?;
            }
        }
        Ok(format!("jsonb_build_array(og_edge_json({u}.eid))"))
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
            self.constrain(format!("{lhs} = {rhs}"));
        }
        Ok(())
    }

    // ----------------------------------------------------------------------
    // Expressions
    // ----------------------------------------------------------------------

    /// SQL for a property access, plus the column's SQL type when known.
    ///
    /// `_id` is the element's identifier. `id` is an ordinary property, as in
    /// Neo4j — whose identifier is `id(n)` / `elementId(n)`, never `n.id`.
    /// Stored types used to read `n.id` as the identifier: a node created with
    /// `{id: 'singleton'}` came back with a number, and `MATCH (n:L {id: $id})`
    /// failed with "invalid input syntax for type bigint". Virtual types already
    /// read it as their source column; now every type does.
    pub fn prop_sql(&self, alias: &str, tid: Option<i32>, prop: &str) -> (String, Option<String>) {
        if prop == "_id" {
            return (format!("{alias}.id"), Some("int8".into()));
        }
        if let Some(t) = tid {
            // A column of the view, or — over a wide view — a typed read of
            // the packed value (views::Layout).
            if let Some((sql, dt)) = views::view_layout(t).read(alias, prop) {
                return (sql, Some(dt));
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

    /// SQL for a property access that must keep its JSON type.
    ///
    /// `->>` yields text, so a number stored as `20` comes back as the string
    /// `"20"` — Neo4j returns an integer, and any client doing arithmetic on
    /// the result sees the difference. `->` keeps the jsonb value as it was
    /// written. Declared properties are real columns and already carry their
    /// type, so they only need wrapping.
    pub fn prop_sql_json(&self, alias: &str, tid: Option<i32>, prop: &str) -> String {
        if prop == "_id" {
            return format!("to_jsonb({alias}.id)");
        }
        if let Some(t) = tid {
            if let Some(sql) = views::view_layout(t).read_json(alias, prop) {
                return sql;
            }
            return format!("({alias}.__ext -> {})", sql_str(prop));
        }
        format!("(og_node_json({alias}.id) -> {})", sql_str(prop))
    }

    /// An expression as it should appear in a result.
    ///
    /// Only property reads differ from `expr`: everything else already has a
    /// SQL type that `jsonb_build_object` renders faithfully.
    fn expr_for_output(&mut self, e: &Expr) -> CResult<String> {
        match e {
            // A parameter keeps its JSON type: `RETURN $n` is a number and
            // `WITH $row AS r` a map (it was the map's text, so `r.name` was null).
            Expr::Param(p) => return Ok(format!("({PARAM} -> {})", sql_str(p))),
            Expr::Func { name, args, .. }
                if name.eq_ignore_ascii_case("coalesce") && self.coalesce_as_jsonb(args, true) =>
            {
                return self.coalesce_jsonb(args);
            }
            _ => {}
        }
        if let Expr::Prop(base, prop) = e {
            if let Some(sql) = self.temporal_field(base, prop, true)? {
                return Ok(sql);
            }
            if !matches!(&**base, Expr::Var(_)) {
                let j = self.jsonb_of(base)?;
                return Ok(format!("(({j}) -> {})", sql_str(prop)));
            }
            if let Expr::Var(v) = &**base {
                match self.binds.get(v).cloned() {
                    Some(Bind::Node { alias, tid }) => {
                        return Ok(self.prop_sql_json(&alias, tid, prop))
                    }
                    Some(Bind::Rel { alias: Some(al), tid, .. }) => {
                        return Ok(self.prop_sql_json(&al, tid, prop))
                    }
                    Some(Bind::Scalar { sql }) => {
                        return Ok(format!("(({sql}) -> {})", sql_str(prop)))
                    }
                    _ => {}
                }
            }
        }
        self.expr(e, None)
    }

    /// The identifier of a bound element.
    ///
    /// A binding may be a pattern variable with a real column, or a scalar
    /// holding the jsonb form of an element — that is what a procedure yields,
    /// and what an UNWIND over collected nodes produces. Both are identifiable;
    /// only the route to the number differs.
    fn element_id_sql(&self, v: &str, fname: &str) -> CResult<String> {
        match self.binds.get(v) {
            Some(Bind::Node { alias, .. }) => Ok(format!("{alias}.id")),
            Some(Bind::Rel { alias: Some(al), .. }) => Ok(format!("{al}.id")),
            Some(Bind::Rel { id_expr, .. }) => Ok(id_expr.clone()),
            Some(Bind::Scalar { sql }) => Ok(format!("(({sql}) ->> '_id')::int8")),
            _ => Err(format!("{fname}() cannot be applied to '{v}'")),
        }
    }

    /// The type id of a bound element, by the same reasoning as `element_id_sql`.
    fn type_id_sql(&self, v: &str, fname: &str) -> CResult<String> {
        match self.binds.get(v) {
            Some(Bind::Node { alias, .. }) => Ok(format!("{alias}.type_id")),
            Some(Bind::Rel { alias: Some(al), .. }) => Ok(format!("{al}.type_id")),
            Some(Bind::Rel { id_expr, .. }) => Ok(format!("og_id_type({id_expr})")),
            Some(Bind::Scalar { sql }) => {
                Ok(format!("og_id_type((({sql}) ->> '_id')::int8)"))
            }
            _ => Err(format!("{fname}() cannot be applied to '{v}'")),
        }
    }

    /// Put back whatever a comprehension binder shadowed, or remove the binder
    /// if it shadowed nothing.
    fn restore_bind(&mut self, var: &str, shadowed: Option<Bind>) {
        match shadowed {
            Some(b) => {
                self.binds.insert(var.to_string(), b);
            }
            None => {
                self.binds.remove(var);
            }
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
                // Packed properties of a wide view are already in `__ext`.
                let props = views::view_layout(t).columns;
                let mut pairs = vec![
                    format!("'_id', {alias}.id"),
                    format!("'_type', og_type_name({alias}.type_id)"),
                ];
                for (name, (col, _)) in &props {
                    pairs.push(format!("{}, to_jsonb({alias}.{col})", sql_str(name)));
                }
                // An OPTIONAL MATCH that found nothing leaves the alias all
                // NULL, and the node is then null — not an empty object, which
                // `count(n)` would count and a client would take for a node.
                format!(
                    "(CASE WHEN {alias}.id IS NULL THEN NULL ELSE \
                     jsonb_strip_nulls({}) || COALESCE({alias}.__ext,'{{}}'::jsonb) END)",
                    views::json_object(&pairs)
                )
            }
            None => format!("og_node_json({alias}.id)"),
        }
    }

    /// The node a pattern alias stands for, as jsonb.
    fn alias_json(&self, alias: &str) -> String {
        let tid = self.alias_tid.get(alias).copied().flatten();
        self.node_json(alias, tid)
    }

    fn rel_json(&self, alias: &str, tid: i32) -> String {
        let props = views::view_layout(tid).columns;
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
            "(CASE WHEN {alias}.id IS NULL THEN NULL ELSE \
             jsonb_strip_nulls({}) || COALESCE({alias}.__ext,'{{}}'::jsonb) END)",
            views::json_object(&pairs)
        )
    }

    /// An element of a list or map literal, wrapped for `to_jsonb()`.
    ///
    /// String and NULL literals compile to bare SQL literals, which carry the
    /// `unknown` type until something resolves them — and `to_jsonb()` is
    /// polymorphic, so it cannot. `['Neo']` would fail to compile without the
    /// explicit type here.
    ///
    /// A property or a parameter keeps its JSON type (`jsonb_of`): `[x IN
    /// nodes(p) | x.i]` gave `["1", "2"]` where Neo4j gives `[1, 2]`.
    fn jsonb_arg(&mut self, e: &Expr) -> CResult<String> {
        self.jsonb_of(e)
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
            // A scalar binding — an UNWIND alias or a comprehension binder —
            // holds jsonb. Compared against something with a known SQL type it
            // has to be read out at that type first, or PostgreSQL sees
            // `jsonb <> text` and refuses.
            Expr::Var(v) => {
                let sql = self.var_value(v)?;
                match (hint, self.binds.get(v)) {
                    (Some(t), Some(Bind::Scalar { .. })) if t != "jsonb" => {
                        format!("(({sql}) #>> '{{}}')::{t}")
                    }
                    _ => sql,
                }
            }
            Expr::Prop(base, p) => {
                if let Some(sql) = self.temporal_field(base, p, false)? {
                    return Ok(sql);
                }
                // A property of a node carried through WITH, read at the type
                // the node's view gives it rather than as text.
                if let Expr::Var(v) = &**base {
                    if let (Some(Bind::Scalar { sql }), Some(ty)) =
                        (self.binds.get(v).cloned(), self.carried_prop_type(v, p))
                    {
                        if is_scalar_type(&ty) && ty != "text" {
                            return Ok(format!("(({sql} ->> {})::{ty})", sql_str(p)));
                        }
                    }
                }
                // A property of any other value — a map parameter (`$row.name`),
                // a map, `coalesce(x1, x2).name` over OPTIONAL MATCH results,
                // `head(list).name` — is a key of its jsonb form.
                let Expr::Var(v) = &**base else {
                    let j = self.jsonb_of(base)?;
                    let key = sql_str(p);
                    return Ok(match hint {
                        Some("jsonb") => format!("(({j}) -> {key})"),
                        Some(t) if t != "text" && is_scalar_type(t) => format!("((({j}) ->> {key})::{t})"),
                        _ => format!("(({j}) ->> {key})"),
                    });
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
            Expr::Neg(a) => match self.type_of(a) {
                Some(t) if is_numeric_type(&t) => format!("(- {})", self.expr(a, Some(&t))?),
                // jsonb or text: read as a number, as arithmetic does.
                None => format!("(- {})", self.numeric_read(a)?),
                Some(t) if t == "text" => format!("(- {})", self.numeric_read(a)?),
                _ => format!("(- {})", self.expr(a, None)?),
            },
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
                views::json_array(&items)
            }
            Expr::Map(kv) => {
                let mut pairs = Vec::new();
                for (k, v) in kv {
                    pairs.push(format!("{}, {}", sql_str(k), self.jsonb_arg(v)?));
                }
                views::json_object(&pairs)
            }
            // `[x IN xs WHERE p | e]` — a scalar subquery over the unnested
            // list. The binder is an ordinary scalar binding while the body
            // compiles, then goes out of scope, so a comprehension cannot leak
            // a variable into the enclosing query.
            Expr::ListComp { var, source, filter, project } => {
                let src = self.expr(source, None)?;
                let a = self.fresh("lc");
                let carried = self.forget_carried(var);
                let shadowed = self
                    .binds
                    .insert(var.clone(), Bind::Scalar { sql: format!("{a}.v") });
                let body = (|slf: &mut Self| -> CResult<(String, String)> {
                    let proj = match project {
                        Some(p) => slf.jsonb_arg(p)?,
                        None => format!("{a}.v"),
                    };
                    let cond = match filter {
                        Some(f) => format!(" WHERE {}", slf.expr(f, Some("bool"))?),
                        None => String::new(),
                    };
                    Ok((proj, cond))
                })(self);
                self.restore_bind(var, shadowed);
                self.restore_carried(var, carried);
                let (proj, cond) = body?;
                format!(
                    "(SELECT coalesce(jsonb_agg({proj}), '[]'::jsonb) \
                       FROM jsonb_array_elements(({src})::jsonb) AS {a}(v){cond})"
                )
            }
            Expr::ListPred { kind, var, source, filter } => {
                let src = self.expr(source, None)?;
                let a = self.fresh("lp");
                let carried = self.forget_carried(var);
                let shadowed = self
                    .binds
                    .insert(var.clone(), Bind::Scalar { sql: format!("{a}.v") });
                let cond = self.expr(filter, Some("bool"));
                self.restore_bind(var, shadowed);
                self.restore_carried(var, carried);
                let cond = cond?;
                let from =
                    format!("FROM jsonb_array_elements(({src})::jsonb) AS {a}(v) WHERE {cond}");
                match kind {
                    ListPredKind::Any => format!("(EXISTS (SELECT 1 {from}))"),
                    ListPredKind::None => format!("(NOT EXISTS (SELECT 1 {from}))"),
                    ListPredKind::Single => format!("((SELECT count(*) {from}) = 1)"),
                    // `all` is "no counterexample", which is also how it treats
                    // the empty list — true, as Cypher says.
                    ListPredKind::All => format!(
                        "(NOT EXISTS (SELECT 1 FROM jsonb_array_elements(({src})::jsonb) \
                          AS {a}(v) WHERE NOT coalesce({cond}, false)))"
                    ),
                }
            }
            // `n { .name, total: count(x), .* }` — a map built from an element.
            // Neo4j's `.*` means "every property", which here is the element's
            // json minus the identity fields the engine adds to it.
            Expr::MapProjection { var, items } => {
                let mut pairs: Vec<String> = Vec::new();
                let mut base: Option<String> = None;
                for item in items {
                    match item {
                        MapProjItem::All => {
                            let whole = self.var_value(var)?;
                            base = Some(format!(
                                "(({whole}) - '_id' - '_type' - '_src' - '_dst')"
                            ));
                        }
                        MapProjItem::Prop(p) => {
                            let target = Expr::Prop(Box::new(Expr::Var(var.clone())), p.clone());
                            let sql = self.expr_for_output(&target)?;
                            pairs.push(format!("{}, {sql}", sql_str(p)));
                        }
                        MapProjItem::Entry(k, e) => {
                            pairs.push(format!("{}, {}", sql_str(k), self.jsonb_arg(e)?));
                        }
                    }
                }
                let built = if pairs.is_empty() {
                    "'{}'::jsonb".to_string()
                } else {
                    views::json_object(&pairs)
                };
                match base {
                    // Explicit entries win over `.*`, as in Cypher.
                    Some(b) => format!("({b} || {built})"),
                    None => built,
                }
            }
            Expr::Case { operand, whens, else_ } => {
                // Results SQL cannot give one type — a list beside a parameter
                // (`CASE WHEN $x IS NULL THEN [null] ELSE $x END`), a node
                // beside a string — are all taken as jsonb.
                let results: Vec<Expr> =
                    whens.iter().map(|(_, v)| v.clone()).chain(else_.iter().map(|e| (**e).clone())).collect();
                let as_jsonb = self.coalesce_as_jsonb(&results, false);
                let head = match operand {
                    Some(o) => format!("CASE {} ", self.expr_own(o)?),
                    None => "CASE ".into(),
                };
                let mut body = String::new();
                for (c, v) in whens {
                    let value = if as_jsonb { self.jsonb_of(v)? } else { self.expr_own(v)? };
                    body.push_str(&format!("WHEN {} THEN {} ", self.expr_own(c)?, value));
                }
                let tail = match else_ {
                    Some(e) if as_jsonb => format!("ELSE {} ", self.jsonb_of(e)?),
                    Some(e) => format!("ELSE {} ", self.expr_own(e)?),
                    None => String::new(),
                };
                format!("({head}{body}{tail}END)")
            }
            Expr::Func { name, args, distinct } => {
                let sql = self.func(name, args, *distinct)?;
                // A list element is jsonb; compared with a typed value it is
                // read out at that type, as a carried scalar is.
                let element = matches!(
                    name.to_ascii_lowercase().as_str(),
                    "head" | "last" | "element_at"
                ) || (name.eq_ignore_ascii_case("coalesce") && self.coalesce_as_jsonb(args, false));
                match hint {
                    Some(t) if element && is_scalar_type(t) => format!("(({sql}) #>> '{{}}')::{t}"),
                    _ => sql,
                }
            }
            Expr::Slice { list, from, to } => self.slice(list, from.as_deref(), to.as_deref())?,
            Expr::HasLabels { var, alts } => self.has_labels(var, alts)?,
            Expr::Reduce { acc, init, var, list, expr } => self.reduce(acc, init, var, list, expr)?,
            Expr::Subquery { kind, patterns, where_, project } => {
                self.pattern_subquery(*kind, patterns, where_.as_deref(), project.as_deref())?
            }
            Expr::Star => "*".into(),
        })
    }

    /// An expression at its own type: a value carried through WITH reads out
    /// of jsonb as what it was (`expr` with no hint leaves it jsonb).
    fn expr_own(&mut self, e: &Expr) -> CResult<String> {
        let t = self.type_of(e);
        self.expr(e, t.as_deref())
    }

    /// The jsonb form of a value, whatever SQL type it compiles to. Property
    /// reads keep their JSON type (`->`), so a number stays a number.
    fn jsonb_of(&mut self, e: &Expr) -> CResult<String> {
        Ok(match e {
            Expr::Param(p) => format!("({PARAM} -> {})", sql_str(p)),
            Expr::Lit(Lit::Null) => "'null'::jsonb".into(),
            Expr::Lit(Lit::Str(s)) => format!("to_jsonb({}::text)", sql_str(s)),
            Expr::Prop(..) => format!("to_jsonb({})", self.expr_for_output(e)?),
            _ => format!("to_jsonb({})", self.expr(e, None)?),
        })
    }

    /// A value whose type is not known to be a number — jsonb carried through
    /// WITH or UNWIND, an undeclared property, a text column — read as one.
    /// A number or a numeric string is read; anything else is the type error
    /// PostgreSQL reports for it.
    fn numeric_read(&mut self, e: &Expr) -> CResult<String> {
        Ok(format!("(({}) #>> '{{}}')::numeric", self.jsonb_of(e)?))
    }

    /// `list[from..to]`: from inclusive, to exclusive, either left out, a
    /// negative bound counted from the end, out-of-range bounds clamped. A
    /// null list or bound gives null.
    fn slice(&mut self, list: &Expr, from: Option<&Expr>, to: Option<&Expr>) -> CResult<String> {
        let l = self.jsonb_of(list)?;
        let mut bound = |b: Option<&Expr>| -> CResult<String> {
            Ok(match b {
                None => "NULL::int8".into(),
                Some(e) => format!("({})::int8", self.expr(e, Some("int8"))?),
            })
        };
        let (f, t) = (bound(from)?, bound(to)?);
        let s = self.fresh("sl");
        let e = self.fresh("se");
        // Ordinals are 1-based; bounds are 0-based positions.
        let pos = |b: &str| {
            format!("(CASE WHEN {s}.{b} < 0 THEN jsonb_array_length({s}.l) + {s}.{b} ELSE {s}.{b} END)")
        };
        let mut keep = Vec::new();
        let mut nulls = vec![format!("{s}.l IS NULL")];
        if from.is_some() {
            keep.push(format!("{e}.i > {}", pos("f")));
            nulls.push(format!("{s}.f IS NULL"));
        }
        if to.is_some() {
            keep.push(format!("{e}.i <= {}", pos("t")));
            nulls.push(format!("{s}.t IS NULL"));
        }
        let filter = if keep.is_empty() { String::new() } else { format!(" WHERE {}", keep.join(" AND ")) };
        Ok(format!(
            "(SELECT CASE WHEN {} THEN NULL ELSE coalesce((SELECT jsonb_agg({e}.v ORDER BY {e}.i) \
             FROM jsonb_array_elements({s}.l) WITH ORDINALITY AS {e}(v, i){filter}), '[]'::jsonb) END \
             FROM (SELECT ({l})::jsonb AS l, {f} AS f, {t} AS t) {s})",
            nulls.join(" OR ")
        ))
    }

    /// `n:A`, `n:A:B`, `n:A|B` as a predicate. A label is carried by a node
    /// whose type is that type or under it — the same reading `labels(n)` and
    /// `MATCH (n:A)` give. The set of types is decided here, so what runs is
    /// a comparison against constants (folded away for a virtual node, whose
    /// type is one). A null element gives null, as in Cypher.
    fn has_labels(&mut self, var: &str, alts: &[Vec<String>]) -> CResult<String> {
        if !self.binds.contains_key(var) {
            return Err(format!("variable '{var}' is not defined in this query"));
        }
        if matches!(self.binds.get(var), Some(Bind::Path { .. })) {
            return Err(format!("'{var}' is a path; a label predicate needs a node or a relationship"));
        }
        let tid_sql = self.type_id_sql(var, "labels")?;
        let bound = match self.binds.get(var) {
            Some(Bind::Node { tid, .. }) => *tid,
            Some(Bind::Scalar { .. }) => self.carried_tid.get(var).copied(),
            _ => None,
        };
        let mut any = Vec::new();
        for alt in alts {
            let mut all = Vec::new();
            for label in alt {
                let term = match types::try_type_id(self.gid, label) {
                    None => "false".to_string(),
                    Some(lt) => {
                        let mut ids = crate::catalog::labeling::og_subtypes(lt);
                        // An umbrella label beside a virtual type is read the
                        // way MATCH (n:_Entity:Customer) reads it.
                        if let Some(b) = bound {
                            if !ids.contains(&b)
                                && crate::fabric::virtual_source(b).is_some()
                                && is_umbrella(lt)
                            {
                                ids.push(b);
                            }
                        }
                        if ids.is_empty() {
                            "false".to_string()
                        } else {
                            let list = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
                            format!("{tid_sql} = ANY('{{{list}}}'::int4[])")
                        }
                    }
                };
                all.push(term);
            }
            any.push(format!("({})", all.join(" AND ")));
        }
        Ok(format!("(CASE WHEN {tid_sql} IS NULL THEN NULL ELSE ({}) END)", any.join(" OR ")))
    }

    /// `reduce(acc = init, x IN list | step)`: a fold, as a recursive query
    /// over the list's positions. The accumulator and the element are jsonb
    /// bindings while `step` compiles; the accumulator is read at the type its
    /// initial value has (a number as numeric, a string as text), so `acc + x`
    /// adds or concatenates as Cypher would.
    fn reduce(&mut self, acc: &str, init: &Expr, var: &str, list: &Expr, step: &Expr) -> CResult<String> {
        if acc == var {
            return Err("reduce(): the accumulator and the element need different names".into());
        }
        let l = self.jsonb_of(list)?;
        let start = self.jsonb_of(init)?;
        let r = self.fresh("rd");
        let acc_type = match self.type_of(init).as_deref() {
            Some(t) if is_numeric_type(t) => Some("numeric".to_string()),
            Some(t) if is_scalar_type(t) => Some(t.to_string()),
            _ => None,
        };
        let acc_carried = self.forget_carried(acc);
        let acc_shadowed = self.binds.insert(acc.to_string(), Bind::Scalar { sql: format!("{r}.acc") });
        let var_carried = self.forget_carried(var);
        let var_shadowed = self.binds.insert(var.to_string(), Bind::Scalar { sql: format!("(({l}) -> {r}.i)") });
        if let Some(t) = &acc_type {
            self.scalar_types.insert(acc.to_string(), t.clone());
        }
        let body = self.jsonb_of(step);
        self.scalar_types.remove(acc);
        self.restore_bind(var, var_shadowed);
        self.restore_carried(var, var_carried);
        self.restore_bind(acc, acc_shadowed);
        self.restore_carried(acc, acc_carried);
        let body = body?;
        Ok(format!(
            "(CASE WHEN jsonb_typeof(({l})::jsonb) = 'array' THEN (WITH RECURSIVE {r}(i, acc) AS (\
             SELECT 0, ({start})::jsonb UNION ALL \
             SELECT {r}.i + 1, ({body})::jsonb FROM {r} WHERE {r}.i < jsonb_array_length(({l})::jsonb)) \
             SELECT {r}.acc FROM {r} ORDER BY {r}.i DESC LIMIT 1) END)"
        ))
    }

    /// A pattern as a value — `EXISTS { … }`, a pattern predicate, `COUNT { … }`,
    /// a pattern comprehension: a correlated subquery. The pattern is compiled
    /// by a compiler that sees this one's bindings, so a variable bound outside
    /// is the same alias inside (that is the correlation), and nothing the
    /// pattern binds leaks out.
    fn pattern_subquery(
        &mut self,
        kind: SubqueryKind,
        patterns: &[Pattern],
        where_: Option<&Expr>,
        project: Option<&Expr>,
    ) -> CResult<String> {
        let mut child = Compiler {
            graph: self.graph.clone(),
            gid: self.gid,
            binds: self.binds.clone(),
            from: Vec::new(),
            wheres: Vec::new(),
            ctes: Vec::new(),
            n: self.n,
            rel_ids: Vec::new(),
            notes: Vec::new(),
            optional: None,
            reachability_only: false,
            alias_tid: self.alias_tid.clone(),
            unmatchable: self.unmatchable.clone(),
            carried_tid: self.carried_tid.clone(),
            scalar_types: self.scalar_types.clone(),
            last_var_hop: None,
            carried_lists: self.carried_lists.clone(),
            last_yields: Vec::new(),
        };
        let compiled = (|c: &mut Compiler| -> CResult<String> {
            c.compile_match(patterns, false, where_)?;
            Ok(match kind {
                SubqueryKind::Exists => "1".into(),
                SubqueryKind::Count => "count(*)".into(),
                SubqueryKind::Collect => {
                    let value = match project {
                        Some(e) => c.jsonb_of(e)?,
                        None => {
                            let last = patterns.last().and_then(|p| match p.elems.last() {
                                Some(PatElem::Node(NodePat { var: Some(v), .. })) => Some(v.clone()),
                                _ => None,
                            });
                            let Some(v) = last else {
                                return Err("a pattern comprehension needs `| expression`".into());
                            };
                            format!("to_jsonb({})", c.var_value(&v)?)
                        }
                    };
                    format!("coalesce(jsonb_agg({value}), '[]'::jsonb)")
                }
            })
        })(&mut child);
        self.n = child.n;
        self.notes.append(&mut child.notes);
        let select = compiled?;
        if !child.ctes.is_empty() {
            self.ctes.append(&mut child.ctes);
        }
        // The first relation of the subquery's FROM may be the hop out of an
        // outer node, written as a join onto what came before it.
        if let Some(first) = child.from.first_mut() {
            if let Some(rest) = first.strip_prefix("CROSS JOIN ") {
                *first = rest.to_string();
            }
        }
        let from = if child.from.is_empty() { String::new() } else { format!(" FROM {}", child.from.join("\n  ")) };
        let wheres = if child.wheres.is_empty() { String::new() } else { format!(" WHERE {}", child.wheres.join(" AND ")) };
        Ok(match kind {
            SubqueryKind::Exists => format!("(EXISTS (SELECT 1{from}{wheres}))"),
            _ => format!("(SELECT {select}{from}{wheres})"),
        })
    }

    /// The declared type of `p` on the node a carried name stands for.
    fn carried_prop_type(&self, v: &str, p: &str) -> Option<String> {
        let t = *self.carried_tid.get(v)?;
        views::view_properties(t).get(p).map(|(_, dt)| dt.clone())
    }

    fn binary(&mut self, op: BinOp, l: &Expr, r: &Expr) -> CResult<String> {
        // Type-directed coercion: comparing a typed column to a parameter or
        // literal casts the right-hand side to the column's type, so the index
        // on that column stays usable.
        // Each side is compiled with the *other* side's type as the hint, so a
        // parameter compared against a typed column is cast to that column's
        // type and the index on it stays usable.
        // `x IN xs` needs the right-hand side as jsonb whatever it came from —
        // a parameter arrives as text and containment is a jsonb operator.
        if op == BinOp::In {
            // A typed column against a list of literals is SQL's own `IN`: a
            // B-tree serves it, and a foreign server receives it as written —
            // jsonb containment is evaluated here, after every row has arrived.
            if let (Some(t), Expr::List(items)) = (self.type_of(l), r) {
                let literal = |e: &Expr| matches!(e, Expr::Lit(x) if !matches!(x, Lit::Null));
                // A boolean against 0/1: the booleans they stand for (see
                // `bool_against_number`); any other number is never equal.
                if t == "bool" && !items.is_empty() && items.iter().all(|e| matches!(e, Expr::Lit(Lit::Int(_) | Lit::Bool(_)))) {
                    let ls = self.expr(l, Some("bool"))?;
                    let mut vs: Vec<&str> = items
                        .iter()
                        .filter_map(|e| match e {
                            Expr::Lit(Lit::Int(1)) | Expr::Lit(Lit::Bool(true)) => Some("true"),
                            Expr::Lit(Lit::Int(0)) | Expr::Lit(Lit::Bool(false)) => Some("false"),
                            _ => None,
                        })
                        .collect();
                    vs.dedup();
                    return Ok(if vs.is_empty() {
                        format!("(CASE WHEN {ls} IS NULL THEN NULL ELSE false END)")
                    } else {
                        format!("({ls} IN ({}))", vs.join(", "))
                    });
                }
                if !items.is_empty() && items.iter().all(literal) {
                    let ls = self.expr(l, Some(&t))?;
                    let mut vs = Vec::with_capacity(items.len());
                    for it in items {
                        vs.push(self.expr(it, Some(&t))?);
                    }
                    return Ok(format!("({ls} IN ({}))", vs.join(", ")));
                }
            }
            // The left side as jsonb, as written: a bare string literal has no
            // SQL type for `to_jsonb()` to take (`'X' IN labels(n)` failed with
            // "could not determine polymorphic type"), and a parameter read as
            // text made `$n IN [1, 2]` look for the string "1".
            let ls = self.jsonb_of(l)?;
            let rs = self.expr(r, Some("jsonb"))?;
            return Ok(format!("(({rs}) @> {ls})"));
        }

        // String predicates work on text, so both sides are compiled *as* text
        // rather than cast afterwards. That distinction matters: a yielded or
        // unwound value is jsonb underneath, and `jsonb::text` keeps the JSON
        // quotes — `"Doc"` rather than `Doc` — so the comparison silently
        // matched nothing. Asking for text here makes the existing scalar
        // read-out (`#>> '{}'`) do the unquoting, and it is also the only form
        // that works for an untyped parameter.
        if matches!(
            op,
            BinOp::StartsWith | BinOp::EndsWith | BinOp::Contains | BinOp::Regex | BinOp::Concat
        ) {
            let ls = self.expr(l, Some("text"))?;
            let rs = self.expr(r, Some("text"))?;
            return Ok(match op {
                BinOp::StartsWith => format!("(({ls})::text LIKE ({rs})::text || '%')"),
                BinOp::EndsWith => format!("(({ls})::text LIKE '%' || ({rs})::text)"),
                BinOp::Contains => format!("(strpos(({ls})::text, ({rs})::text) > 0)"),
                BinOp::Regex => format!("(({ls})::text ~ ({rs})::text)"),
                _ => format!("(({ls})::text || ({rs})::text)"),
            });
        }

        // Each side's own type, and the type it is compiled at: the other
        // side's, so a parameter compared with a typed column is cast to the
        // column's type (and its index stays usable) — or its own when the
        // other side's is unknown, so a value carried through WITH reads out.
        let lt = self.type_of(l);
        let rt = self.type_of(r);
        if matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod) {
            if let Some(sql) = self.arithmetic(op, l, r, lt.as_deref(), rt.as_deref())? {
                return Ok(sql);
            }
        }
        if matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge) {
            if let Some(sql) = self.bool_against_number(op, l, r, lt.as_deref(), rt.as_deref())? {
                return Ok(sql);
            }
        }
        // A value carried as jsonb (a WITH or YIELD column, an UNWIND element)
        // against a parameter or another such value, neither with a known
        // type: compared as JSON. As text against jsonb it was "operator does
        // not exist: jsonb = text" (`SHOW INDEXES YIELD name WHERE name = $n`).
        if lt.is_none()
            && rt.is_none()
            && matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
            && (self.jsonb_valued(l) || self.jsonb_valued(r))
        {
            let (a, b) = (self.jsonb_of(l)?, self.jsonb_of(r)?);
            return Ok(match op {
                BinOp::Eq => format!("({a} = {b})"),
                BinOp::Ne => format!("({a} <> {b})"),
                BinOp::Lt => format!("({a} < {b})"),
                BinOp::Le => format!("({a} <= {b})"),
                BinOp::Gt => format!("({a} > {b})"),
                _ => format!("({a} >= {b})"),
            });
        }
        let mut ls = self.expr(l, rt.as_deref().or(lt.as_deref()))?;
        let mut rs = self.expr(r, lt.as_deref().or(rt.as_deref()))?;
        let (lhint, rhint) = (rt.clone(), lt.clone());

        // An undeclared property reads out of jsonb as text. Compared against a
        // number that is `text = integer`, which PostgreSQL rejects outright —
        // so read the untyped side at the other side's type instead.
        if let (None, Some(t)) = (&rhint, &lhint) {
            if matches!(l, Expr::Prop(..)) && t.as_str() != "text" {
                ls = format!("({ls})::{t}");
            }
        }
        if let (None, Some(t)) = (&lhint, &rhint) {
            if matches!(r, Expr::Prop(..)) && t.as_str() != "text" {
                rs = format!("({rs})::{t}");
            }
        }

        // A source that keeps dates as text, compared with a date or a time:
        // read the text at that type rather than refuse `text >= date`.
        const TEMPORAL: &[&str] = &["date", "timestamptz", "timestamp"];
        if let (Some(lt), Some(rt)) = (&rhint, &lhint) {
            if lt == "text" && TEMPORAL.contains(&rt.as_str()) && matches!(l, Expr::Prop(..)) {
                ls = format!("({ls})::{rt}");
            }
            if rt == "text" && TEMPORAL.contains(&lt.as_str()) && matches!(r, Expr::Prop(..)) {
                rs = format!("({rs})::{lt}");
            }
        }

        // Against a constant, plain `=` is both Cypher's answer (null = 'x' is
        // null, not false) and the form a source database can use an index for
        // — `IS NOT DISTINCT FROM` reaches a foreign server intact but no
        // B-tree serves it, and a MySQL-dialect hop renders it as `<=>`.
        let constant = |e: &Expr| matches!(e, Expr::Param(_)) || matches!(e, Expr::Lit(x) if !matches!(x, Lit::Null));
        let plain = constant(l) || constant(r);

        // A date moved by a duration is still a date in Cypher; SQL's
        // `date ± interval` is a timestamp.
        let date_shift = rhint.as_deref() == Some("date") && lhint.as_deref() == Some("interval");

        Ok(match op {
            BinOp::Eq if plain => format!("({ls} = {rs})"),
            BinOp::Ne if plain => format!("({ls} <> {rs})"),
            BinOp::Add if date_shift => format!("(({ls} + {rs})::date)"),
            BinOp::Sub if date_shift => format!("(({ls} - {rs})::date)"),
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
            BinOp::Concat => unreachable!("handled above"),
            // `::text` on a jsonb string keeps its quotes — `"c_doc"`, not
            // `c_doc` — so a jsonb operand never matched a plain one. It only
            // looked right when the other side was a literal, because the
            // literal's type propagated a hint that read the jsonb out first;
            // the same predicate against a parameter compared `"c_doc"` with
            // `c_d%` and quietly returned nothing.
            //
            // `as_text` reads either representation out to the same string, so
            // these no longer depend on a hint arriving.
            BinOp::StartsWith
            | BinOp::EndsWith
            | BinOp::Contains
            | BinOp::Regex => unreachable!("handled above"),
            BinOp::In => unreachable!("handled above"),
        })
    }

    /// `+ - * / %` over numbers whose SQL type is not known to be one.
    ///
    /// A value carried through WITH or UNWIND is jsonb and an undeclared
    /// property is text, and SQL has no `jsonb * jsonb` — so `WITH o RETURN
    /// sum(o.qty * o.price)` failed with "operator does not exist", and
    /// `WITH o.qty * o.price AS v RETURN sum(v)` with "function sum(jsonb)
    /// does not exist". An operand of unknown type is read as a number here
    /// (see `numeric_read`); typed operands are left as they are, so integer
    /// arithmetic on integer columns stays integer arithmetic.
    ///
    /// `+` with a string concatenates and `+` with a list appends, as in
    /// Cypher. Dates and durations keep the plain SQL operators (`None`).
    fn arithmetic(
        &mut self,
        op: BinOp,
        l: &Expr,
        r: &Expr,
        lt: Option<&str>,
        rt: Option<&str>,
    ) -> CResult<Option<String>> {
        const TEMPORAL: &[&str] = &["date", "timestamptz", "timestamp", "interval"];
        if [lt, rt].iter().any(|t| t.is_some_and(|t| TEMPORAL.contains(&t))) {
            return Ok(None);
        }
        if op == BinOp::Add {
            if self.is_list_value(l) || self.is_list_value(r) {
                let (a, b) = (self.jsonb_of(l)?, self.jsonb_of(r)?);
                return Ok(Some(format!("({a} || {b})")));
            }
            if lt == Some("text") || rt == Some("text") {
                let (a, b) = (self.expr(l, Some("text"))?, self.expr(r, Some("text"))?);
                return Ok(Some(format!("(({a})::text || ({b})::text)")));
            }
        }
        // Each operand, and whether it is an integer: known from its type, or
        // — for one read as numeric — from the value (`scale() = 0`), so that
        // `/` divides integers as Cypher does (7 / 2 is 3) and anything else
        // exactly.
        let int_type = |t: &str| matches!(t, "int2" | "int4" | "int8");
        let side = |slf: &mut Self, e: &Expr, own: Option<&str>, other: Option<&str>| -> CResult<(String, Option<bool>)> {
            Ok(match own {
                Some(t) if is_numeric_type(t) => (slf.expr(e, Some(t))?, Some(int_type(t))),
                None | Some("text") => match e {
                    Expr::Lit(Lit::Null) => ("NULL::numeric".into(), Some(false)),
                    // A parameter beside a typed number is cast to its type,
                    // as before — integer stays integer.
                    Expr::Param(_) if other.is_some_and(is_numeric_type) => {
                        (slf.expr(e, other)?, other.map(int_type))
                    }
                    _ => (slf.numeric_read(e)?, None),
                },
                // Booleans, vectors and the rest: as written.
                Some(_) => (slf.expr(e, None)?, Some(false)),
            })
        };
        let (a, a_int) = side(self, l, lt, rt)?;
        let (b, b_int) = side(self, r, rt, lt)?;
        Ok(Some(match op {
            BinOp::Add => format!("({a} + {b})"),
            BinOp::Sub => format!("({a} - {b})"),
            BinOp::Mul => format!("({a} * {b})"),
            BinOp::Div => match (a_int, b_int) {
                (Some(false), _) | (_, Some(false)) | (Some(true), Some(true)) => {
                    format!("({a} / NULLIF({b}, 0))")
                }
                _ => {
                    let runtime: Vec<String> = [(&a, a_int), (&b, b_int)]
                        .iter()
                        .filter(|(_, k)| k.is_none())
                        .map(|(x, _)| format!("scale({x}) = 0"))
                        .collect();
                    format!(
                        "(CASE WHEN {} THEN trunc({a} / NULLIF({b}, 0)) ELSE {a} / NULLIF({b}, 0) END)",
                        runtime.join(" AND ")
                    )
                }
            },
            _ => format!("(({a})::numeric % NULLIF(({b})::numeric, 0))"),
        }))
    }

    /// A boolean compared with a number: `n.active = 1`.
    ///
    /// SQL refuses `boolean = integer`, and the error used to reach a Bolt
    /// client as `DatabaseNotFound`. Sources that have no boolean type keep
    /// flags as 0/1 (MySQL `tinyint(1)`, SQL Server `bit`), and a query writer
    /// thinking of them writes `= 1`; so a boolean reads as 1 or 0 here. A
    /// literal 0 or 1 becomes `false`/`true` and the comparison stays on the
    /// column, where an index or a source can serve it; any other number
    /// compares against 1 or 0 (`n.active = 2` is never true). The other way
    /// round — a 0/1 number column against `true` — compares it with 1.
    fn bool_against_number(
        &mut self,
        op: BinOp,
        l: &Expr,
        r: &Expr,
        lt: Option<&str>,
        rt: Option<&str>,
    ) -> CResult<Option<String>> {
        let num = |t: Option<&str>| t.is_some_and(is_numeric_type);
        let (b, n, bool_left) = if lt == Some("bool") && num(rt) {
            (l, r, true)
        } else if rt == Some("bool") && num(lt) {
            (r, l, false)
        } else {
            return Ok(None);
        };
        let constant = [l, r].iter().any(|e| matches!(e, Expr::Param(_) | Expr::Lit(_)));
        let sym = match op {
            BinOp::Eq if constant => "=",
            BinOp::Ne if constant => "<>",
            BinOp::Eq => "IS NOT DISTINCT FROM",
            BinOp::Ne => "IS DISTINCT FROM",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            _ => ">=",
        };
        // `n.flag = true` on a 0/1 column: the column against 1, as written.
        if let Expr::Lit(Lit::Bool(v)) = b {
            let ns = self.expr(n, if bool_left { rt } else { lt })?;
            let k = if *v { "1" } else { "0" };
            let (a, c) = if bool_left { (k.to_string(), ns) } else { (ns, k.to_string()) };
            return Ok(Some(format!("({a} {sym} {c})")));
        }
        let bs = self.expr(b, Some("bool"))?;
        if let (Expr::Lit(Lit::Int(k @ (0 | 1))), BinOp::Eq | BinOp::Ne) = (n, op) {
            let lit = if *k == 1 { "true" } else { "false" };
            let (a, c) = if bool_left { (bs, lit.to_string()) } else { (lit.to_string(), bs) };
            return Ok(Some(format!("({a} {sym} {c})")));
        }
        let ns = self.expr(n, if bool_left { rt } else { lt })?;
        let bi = format!("(({bs})::int)");
        let (a, c) = if bool_left { (bi, ns) } else { (ns, bi) };
        Ok(Some(format!("({a} {sym} {c})")))
    }

    /// The SQL type of an expression when we can determine it — used to drive
    /// parameter coercion.
    fn type_of(&self, e: &Expr) -> Option<String> {
        match e {
            Expr::Prop(base, p) => {
                // `d.year` on a date that is not a pattern variable.
                if temporal_unit(p).is_some() {
                    let element = matches!(&**base, Expr::Var(v)
                        if matches!(self.binds.get(v), Some(Bind::Node { .. } | Bind::Rel { .. })));
                    if !element
                        && matches!(self.type_of(base).as_deref(), Some("date" | "timestamptz" | "timestamp"))
                    {
                        return Some("int8".into());
                    }
                }
                let Expr::Var(v) = &**base else { return None };
                match self.binds.get(v) {
                    Some(Bind::Node { alias, tid }) => self.prop_sql(alias, *tid, p).1,
                    Some(Bind::Rel { alias: Some(a), tid, .. }) => self.prop_sql(a, *tid, p).1,
                    Some(Bind::Scalar { .. }) => {
                        self.carried_prop_type(v, p).filter(|t| is_scalar_type(t))
                    }
                    _ => None,
                }
            }
            // What a value was before WITH turned it into jsonb.
            Expr::Var(v) => match self.binds.get(v) {
                Some(Bind::Scalar { .. }) => self.scalar_types.get(v).cloned(),
                _ => None,
            },
            Expr::Lit(Lit::Int(_)) => Some("int8".into()),
            Expr::Lit(Lit::Float(_)) => Some("float8".into()),
            Expr::Lit(Lit::Bool(_)) => Some("bool".into()),
            Expr::Lit(Lit::Str(_)) => Some("text".into()),
            Expr::Neg(a) => match self.type_of(a) {
                Some(t) if is_numeric_type(&t) || t == "interval" => Some(t),
                Some(t) if t != "text" => None,
                _ => Some("numeric".into()),
            },
            Expr::Not(_) | Expr::IsNull(..) | Expr::HasLabels { .. } | Expr::ListPred { .. } => {
                Some("bool".into())
            }
            Expr::Subquery { kind: SubqueryKind::Exists, .. } => Some("bool".into()),
            Expr::Subquery { kind: SubqueryKind::Count, .. } => Some("int8".into()),
            Expr::Binary(op, l, r) => match op {
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                    if *op == BinOp::Add && (self.is_list_value(l) || self.is_list_value(r)) {
                        return None;
                    }
                    arithmetic_type(*op, self.type_of(l).as_deref(), self.type_of(r).as_deref())
                }
                BinOp::Pow => Some("float8".into()),
                BinOp::Concat => Some("text".into()),
                _ => Some("bool".into()),
            },
            // Constructors and conversions say what they make, so the other side
            // of a comparison is read at that type: `o.placed >= date('2016-01-01')`
            // is `date >= date`, which a source can be sent.
            Expr::Func { name, args, .. } => {
                let arg = || args.first().and_then(|a| self.type_of(a));
                match name.to_ascii_lowercase().as_str() {
                    "date" => Some("date".into()),
                    "datetime" => Some("timestamptz".into()),
                    "localdatetime" => Some("timestamp".into()),
                    "duration" => Some("interval".into()),
                    "tointeger" => Some("int8".into()),
                    "tofloat" => Some("float8".into()),
                    "tostring" | "randomuuid" => Some("text".into()),
                    "count" => Some("int8".into()),
                    "size" | "length" => Some("int4".into()),
                    "stdev" | "ceil" | "floor" | "sqrt" => Some("float8".into()),
                    "round" => Some("numeric".into()),
                    "sum" => match arg().as_deref() {
                        Some("float4" | "float8") => Some("float8".into()),
                        Some("int2" | "int4") => Some("int8".into()),
                        Some("interval") => Some("interval".into()),
                        Some(t) if t != "text" && t != "jsonb" && !is_numeric_type(t) => None,
                        _ => Some("numeric".into()),
                    },
                    "avg" => match arg().as_deref() {
                        Some("float4" | "float8") => Some("float8".into()),
                        Some("interval") => Some("interval".into()),
                        Some(t) if t != "text" && t != "jsonb" && !is_numeric_type(t) => None,
                        _ => Some("numeric".into()),
                    },
                    "min" | "max" => arg().filter(|t| t != "jsonb"),
                    "abs" => match arg() {
                        Some(t) if is_numeric_type(&t) => Some(t),
                        _ => Some("numeric".into()),
                    },
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// `x.year`, `x.month` … on a temporal value. `None` when `base` is a
    /// pattern variable — then `.year` is a property like any other — or when
    /// `p` is not a temporal field.
    fn temporal_field(&mut self, base: &Expr, p: &str, for_output: bool) -> CResult<Option<String>> {
        let Some(field) = temporal_unit(p) else { return Ok(None) };
        if let Expr::Var(v) = base {
            match self.binds.get(v) {
                Some(Bind::Scalar { .. }) => {}
                _ => return Ok(None),
            }
        }
        // A column whose type is known: plain `extract`, which a source can be
        // sent when the column is a date.
        if let Some(ty) = self.type_of(base) {
            if matches!(ty.as_str(), "date" | "timestamptz" | "timestamp") {
                // The type as a hint: a value carried through WITH reads out at it.
                let sql = self.expr(base, Some(&ty))?;
                if ty == "date" && matches!(field, "hour" | "minute" | "second" | "millisecond") {
                    return Err(format!("a date has no .{p} — use datetime() for a point in time"));
                }
                let part = extract_sql(&sql, field);
                return Ok(Some(if for_output { format!("to_jsonb({part})") } else { part }));
            }
        }
        // Anything else is decided per row: a map keeps `.year` as a key lookup,
        // a date or datetime string is read as one. Plain SQL, no helper
        // function: a database whose extension was created before this release
        // has no new catalog objects, and compiled Cypher must run there too.
        let j = match base {
            // The parameter as jsonb, so a map parameter is still a map.
            Expr::Param(name) => format!("({PARAM} -> {})", sql_str(name)),
            _ => format!("to_jsonb({})", self.expr(base, None)?),
        };
        let part = extract_sql(&format!("({j} #>> '{{}}')::timestamptz"), field);
        let value = format!(
            "(CASE jsonb_typeof({j}) WHEN 'object' THEN {j} -> {key} \
             WHEN 'string' THEN to_jsonb({part}) END)",
            key = sql_str(p)
        );
        Ok(Some(if for_output { value } else { format!("({value} #>> '{{}}')") }))
    }

    /// `date(…)`, `datetime(…)`, `localdatetime(…)`: now, a string, a map of
    /// fields, or another temporal value.
    fn temporal_ctor(&mut self, ctor: &str, args: &[Expr]) -> CResult<String> {
        let (ty, now) = match ctor {
            "date" => ("date", "current_date"),
            "datetime" => ("timestamptz", "now()"),
            _ => ("timestamp", "localtimestamp"),
        };
        match args {
            [] => Ok(now.into()),
            // A literal is converted once, when the statement is planned — and a
            // constant of the column's type is what a foreign server is sent.
            [Expr::Lit(Lit::Str(s))] => Ok(format!("{}::{ty}", sql_str(s))),
            [Expr::Lit(Lit::Null)] => Ok(format!("NULL::{ty}")),
            [Expr::Map(kv)] => {
                let mut get = |k: &str, default: &str| -> CResult<String> {
                    match kv.iter().find(|(key, _)| key.eq_ignore_ascii_case(k)) {
                        Some((_, v)) => Ok(format!("({})::int", self.expr(v, Some("int8"))?)),
                        None => Ok(default.into()),
                    }
                };
                let year = get("year", "")?;
                if year.is_empty() {
                    return Err(format!("{ctor}({{…}}) needs at least a year"));
                }
                let (month, day) = (get("month", "1")?, get("day", "1")?);
                if ty == "date" {
                    return Ok(format!("make_date({year}, {month}, {day})"));
                }
                let (hour, minute) = (get("hour", "0")?, get("minute", "0")?);
                let second = match kv.iter().find(|(key, _)| key.eq_ignore_ascii_case("second")) {
                    Some((_, v)) => format!("({})::float8", self.expr(v, Some("float8"))?),
                    None => "0".into(),
                };
                let make = if ty == "timestamp" { "make_timestamp" } else { "make_timestamptz" };
                Ok(format!("{make}({year}, {month}, {day}, {hour}, {minute}, {second})"))
            }
            [x] => {
                let have = self.type_of(x);
                if have.as_deref() == Some(ty) {
                    return self.expr(x, Some(ty));
                }
                // A parameter or an unwound value arrives as text or jsonb; ask
                // for text, which reads a jsonb string without its quotes.
                Ok(format!("({})::{ty}", self.expr(x, Some("text"))?))
            }
            _ => Err(format!("{ctor}() takes one argument: a string, a map, or another date or time")),
        }
    }

    /// `duration('P30D')`, `duration({days: 30})` — an SQL interval, which a
    /// date or datetime can be moved by.
    fn duration(&mut self, args: &[Expr]) -> CResult<String> {
        match args {
            [Expr::Lit(Lit::Str(s))] => Ok(format!("{}::interval", sql_str(s))),
            [Expr::Map(kv)] => {
                let mut parts = Vec::new();
                for (k, v) in kv {
                    let (arg, scale) = match k.to_ascii_lowercase().as_str() {
                        "years" => ("years", 1.0),
                        "months" => ("months", 1.0),
                        "quarters" => ("months", 3.0),
                        "weeks" => ("weeks", 1.0),
                        "days" => ("days", 1.0),
                        "hours" => ("hours", 1.0),
                        "minutes" => ("mins", 1.0),
                        "seconds" => ("secs", 1.0),
                        "milliseconds" => ("secs", 0.001),
                        other => return Err(format!("duration() has no field '{other}'")),
                    };
                    let val = self.expr(v, Some("float8"))?;
                    parts.push(if arg == "secs" {
                        format!("secs => ({val})::float8 * {scale}")
                    } else if scale != 1.0 {
                        format!("{arg} => (({val})::int * {})", scale as i64)
                    } else {
                        format!("{arg} => ({val})::int")
                    });
                }
                Ok(format!("make_interval({})", parts.join(", ")))
            }
            [x] => Ok(format!("({})::interval", self.expr(x, Some("text"))?)),
            _ => Err("duration() takes one argument: an ISO 8601 string such as 'P30D', or a map such as {days: 30}".into()),
        }
    }

    /// `toInteger(x)` / `toFloat(x)` as Cypher means them: a number is
    /// truncated (toInteger(2.7) is 2), a string that is not a number gives
    /// null, and what the value is decides that — not what SQL can cast.
    ///
    /// The SQL used to be a bare `::int8`, which rounds 2.5 to 3, fails the
    /// whole query on one non-numeric string, and cannot read a jsonb value at
    /// all ("cannot cast jsonb string to type bigint") — so `sum(toInteger(x))`
    /// over anything read from a map, an UNWIND or an undeclared property failed.
    fn to_number(&mut self, int: bool, args: &[Expr]) -> CResult<String> {
        let [x] = args else {
            return Err(format!("{}() takes one argument", if int { "toInteger" } else { "toFloat" }));
        };
        let target = if int { "int8" } else { "float8" };
        if matches!(x, Expr::Lit(Lit::Null)) {
            return Ok(format!("NULL::{target}"));
        }
        // A string is a number only if it reads as one; then toInteger truncates.
        let from_text = |t: &str| {
            let read = if int {
                format!("trunc(btrim({t})::numeric)::int8")
            } else {
                format!("btrim({t})::float8")
            };
            format!("(CASE WHEN {t} ~ '{NUMERIC_TEXT}' THEN {read} END)")
        };
        let ty = self.type_of(x);
        let sql = self.expr(x, ty.as_deref())?;
        Ok(match ty.as_deref() {
            Some("int2" | "int4" | "int8") => format!("({sql})::{target}"),
            Some("numeric" | "float4" | "float8") if int => format!("trunc({sql})::int8"),
            Some("numeric" | "float4" | "float8") => format!("({sql})::float8"),
            Some("bool") if int => format!("({sql})::int::int8"),
            Some("text") => from_text(&format!("({sql})::text")),
            // jsonb, or a type known only at run time: let the value decide.
            _ => {
                let j = format!("to_jsonb({sql})");
                let number = if int { format!("trunc(({j})::numeric)::int8") } else { format!("({j})::float8") };
                let boolean = if int { format!("(({j})::bool)::int::int8") } else { "NULL::float8".into() };
                format!(
                    "(CASE jsonb_typeof({j}) WHEN 'number' THEN {number} WHEN 'string' THEN {} \
                     WHEN 'boolean' THEN {boolean} END)",
                    from_text(&format!("({j} #>> '{{}}')"))
                )
            }
        })
    }

    /// `toString(x)`. A value carried through WITH or UNWIND is jsonb, and
    /// `jsonb::text` spells a string with its JSON quotes — `"abc"` came back as
    /// `"\"abc\""`. When the type is not known, the value decides.
    fn to_text(&mut self, args: &[Expr]) -> CResult<String> {
        let [x] = args else { return Err("toString() takes one argument".into()) };
        if matches!(x, Expr::Lit(Lit::Null)) {
            return Ok("NULL::text".into());
        }
        let ty = self.type_of(x);
        let sql = self.expr(x, ty.as_deref())?;
        Ok(match ty {
            Some(_) => format!("({sql})::text"),
            None => {
                let j = format!("to_jsonb({sql})");
                format!("(CASE jsonb_typeof({j}) WHEN 'string' THEN {j} #>> '{{}}' WHEN 'null' THEN NULL ELSE ({j})::text END)")
            }
        })
    }

    /// Is this value a list — as written, or a name carried through WITH that
    /// holds one, or a sum of lists?
    fn is_list_value(&self, e: &Expr) -> bool {
        match e {
            Expr::Var(v) => self.carried_lists.contains(v),
            Expr::Binary(BinOp::Add, a, b) => self.is_list_value(a) || self.is_list_value(b),
            Expr::Reduce { init, .. } => self.is_list_value(init),
            Expr::Case { whens, else_, .. } => {
                whens.iter().any(|(_, v)| self.is_list_value(v)) || else_.as_ref().is_some_and(|x| self.is_list_value(x))
            }
            Expr::Func { name, .. } if name.eq_ignore_ascii_case("coalesce") => {
                matches!(e, Expr::Func { args, .. } if args.iter().any(|a| self.is_list_value(a)))
            }
            _ => is_list_expr(e),
        }
    }

    /// Is this value jsonb that has no text form — a list, a map, an element,
    /// a value carried through WITH or UNWIND whose type was not kept?
    fn jsonb_valued(&self, e: &Expr) -> bool {
        match e {
            Expr::List(_) | Expr::Map(_) | Expr::MapProjection { .. } | Expr::ListComp { .. } | Expr::Slice { .. } => true,
            Expr::Reduce { .. } => true,
            Expr::Binary(BinOp::Add, ..) if self.is_list_value(e) => true,
            Expr::Func { name, .. } => {
                is_list_expr(e)
                    || matches!(
                        name.to_ascii_lowercase().as_str(),
                        "properties" | "head" | "last" | "element_at" | "startnode" | "endnode" | "coalesce"
                    ) && self.type_of(e).is_none()
            }
            Expr::Var(v) => match self.binds.get(v) {
                Some(Bind::Node { .. } | Bind::Rel { .. } | Bind::Path { .. }) => true,
                Some(Bind::Scalar { .. }) => !self.scalar_types.contains_key(v),
                None => false,
            },
            _ => false,
        }
    }

    /// Should `coalesce(…)` be computed over jsonb rather than SQL scalars?
    ///
    /// SQL wants one type for every branch. A branch whose type is a known
    /// number, date or boolean sets it (`coalesce(n.qty, 0)`), as before. When
    /// none does, a branch that only jsonb can hold — a list, a map, a node —
    /// makes it jsonb (`coalesce(n.tags, [])` failed with "COALESCE types text
    /// and jsonb cannot be matched"). In a result, where the value ends up as
    /// JSON anyway, a branch of unknown type does too: `coalesce(n.tags, $list)`
    /// returned the list as a JSON *string* when both were read as text.
    fn coalesce_as_jsonb(&self, args: &[Expr], for_output: bool) -> bool {
        let typed = |e: &Expr| self.type_of(e).filter(|t| t != "text" && t != "jsonb");
        if args.iter().any(|x| typed(x).is_some()) {
            return false;
        }
        args.iter().any(|x| self.jsonb_valued(x))
            || (for_output && args.iter().any(|x| self.type_of(x).is_none() && !matches!(x, Expr::Lit(Lit::Null))))
    }

    /// `coalesce` over jsonb: a JSON null is skipped like an SQL one.
    fn coalesce_jsonb(&mut self, args: &[Expr]) -> CResult<String> {
        let mut parts = Vec::new();
        for x in args {
            parts.push(format!("NULLIF({}, 'null'::jsonb)", self.jsonb_of(x)?));
        }
        Ok(format!("coalesce({})", parts.join(", ")))
    }

    fn func(&mut self, name: &str, args: &[Expr], distinct: bool) -> CResult<String> {
        let lname = name.to_ascii_lowercase();
        let d = if distinct { "DISTINCT " } else { "" };

        // Aggregates
        match lname.as_str() {
            "count" => {
                // `count(n)` of a node counts its identifier: the same rows
                // (the node is null exactly when its id is), without building
                // each node's JSON only to count it.
                if let Some(Expr::Var(v)) = args.first() {
                    if let Some(Bind::Node { alias, .. }) = self.binds.get(v) {
                        return Ok(format!("count({d}{alias}.id)"));
                    }
                }
                return Ok(if matches!(args.first(), Some(Expr::Star)) || args.is_empty() {
                    "count(*)".into()
                } else {
                    format!("count({d}{})", self.expr(&args[0], None)?)
                })
            }
            // sum/avg over a value whose type is not known to be a number —
            // jsonb carried through WITH, an undeclared property, text — read
            // it as one: `function sum(jsonb) does not exist` otherwise.
            "sum" | "avg" => {
                let [x] = args else { return Err(format!("{lname}() takes one argument")) };
                let a = match self.type_of(x) {
                    None => self.numeric_read(x)?,
                    Some(t) if t == "text" || t == "jsonb" => self.numeric_read(x)?,
                    Some(t) => self.expr(x, Some(&t))?,
                };
                return Ok(format!("{lname}({d}{a})"));
            }
            // min/max keep the value's own type: strings and dates have them
            // too. jsonb has no min()/max() in SQL, so a value of unknown type
            // is ordered as jsonb is — numbers by value, strings as text.
            "min" | "max" => {
                let [x] = args else { return Err(format!("{lname}() takes one argument")) };
                return Ok(match self.type_of(x) {
                    Some(t) if t != "jsonb" => format!("{lname}({d}{})", self.expr(x, Some(&t))?),
                    _ => {
                        let j = self.jsonb_of(x)?;
                        let dir = if lname == "min" { "" } else { " DESC" };
                        format!(
                            "(array_agg({j} ORDER BY {j}{dir}) FILTER (WHERE jsonb_typeof({j}) <> 'null'))[1]"
                        )
                    }
                });
            }
            "collect" => {
                // Cypher's collect() skips nulls (an OPTIONAL MATCH that found
                // nothing contributes no element) and is [] over no rows.
                let a = self.expr(&args[0], None)?;
                return Ok(format!(
                    "coalesce(jsonb_agg({d}{a}) FILTER (WHERE ({a}) IS NOT NULL), '[]'::jsonb)"
                ));
            }
            "stdev" => {
                let a = match self.type_of(&args[0]) {
                    Some(t) if is_numeric_type(&t) => self.expr(&args[0], Some(&t))?,
                    _ => self.numeric_read(&args[0])?,
                };
                return Ok(format!("stddev_samp(({a})::float8)"));
            }
            // Lists are jsonb arrays; `->` takes a negative index from the end.
            "head" | "last" | "tail" => {
                let [x] = args else { return Err(format!("{name}() takes one argument: a list")) };
                let l = self.jsonb_of(x)?;
                return Ok(match lname.as_str() {
                    "head" => format!("({l} -> 0)"),
                    "last" => format!("({l} -> -1)"),
                    _ => format!("({l} - 0)"),
                });
            }
            "abs" => {
                let [x] = args else { return Err("abs() takes one argument".into()) };
                return Ok(match self.type_of(x) {
                    Some(t) if is_numeric_type(&t) => format!("abs({})", self.expr(x, Some(&t))?),
                    _ => format!("abs({})", self.numeric_read(x)?),
                });
            }
            "date" | "datetime" | "localdatetime" => return self.temporal_ctor(&lname, args),
            "duration" => return self.duration(args),
            "tointeger" => return self.to_number(true, args),
            "tofloat" => return self.to_number(false, args),
            "tostring" => return self.to_text(args),
            _ => {}
        }

        let mut a = Vec::new();
        for x in args {
            a.push(self.expr_own(x)?);
        }

        Ok(match lname.as_str() {
            "id" => {
                let Some(Expr::Var(v)) = args.first() else {
                    return Err("id() expects a pattern variable".into());
                };
                self.element_id_sql(v, "id")?
            }
            // `elementId()` is Neo4j's stable string handle for an element.
            // Ours is the int8 identifier; rendering it as text is the whole
            // difference, and it round-trips through `id()` unchanged.
            "elementid" => {
                let Some(Expr::Var(v)) = args.first() else {
                    return Err("elementId() expects a pattern variable".into());
                };
                let inner = self.element_id_sql(v, "elementId")?;
                format!("({inner})::text")
            }
            // `type(r)` is one name. `labels(n)` is a *list* in Cypher, and here
            // a node genuinely carries every name on its supertype chain — that
            // is what the type hierarchy means. Returning the chain is what lets
            // `'Foo' IN labels(n)` and `[l IN labels(n) …]` mean what they mean
            // in Neo4j against a `(:Super:Sub)`-shaped graph.
            "labels" => {
                let Some(Expr::Var(v)) = args.first() else {
                    return Err("labels() expects a pattern variable".into());
                };
                let tid = self.type_id_sql(v, "labels")?;
                format!(
                    "to_jsonb(ARRAY(SELECT og_type_name(t) FROM unnest(og_supertypes({tid})) AS t))"
                )
            }
            "type" => {
                let Some(Expr::Var(v)) = args.first() else {
                    return Err("type() expects a pattern variable".into());
                };
                let tid = self.type_id_sql(v, "type")?;
                format!("og_type_name({tid})")
            }
            // `size(list)`, `size(string)`; `length(path)` (and, leniently,
            // `length` of a list or string). Decided by the type where it is
            // known and by the value where it is not — a parameter, an
            // undeclared property, a value carried through WITH. Null for null,
            // as everywhere in Cypher: it used to be `jsonb_array_length(text)`,
            // which does not exist, for any property that was not declared.
            "length" | "size" => {
                let [x] = args else { return Err(format!("{name}() takes one argument")) };
                if matches!(x, Expr::Map(_) | Expr::MapProjection { .. }) {
                    return Err(format!("{name}() takes a string or a list; a map has no size in Cypher"));
                }
                match self.type_of(x).as_deref() {
                    Some("text") => format!("char_length(({})::text)", self.expr(x, Some("text"))?),
                    Some(t) if is_scalar_type(t) => {
                        return Err(format!("{name}() takes a string or a list, not a {t}"))
                    }
                    _ => {
                        let j = self.jsonb_of(x)?;
                        format!(
                            "(CASE jsonb_typeof({j}) WHEN 'array' THEN jsonb_array_length({j}) \
                             WHEN 'string' THEN char_length({j} #>> '{{}}') \
                             WHEN 'object' THEN CASE WHEN {j} ? '_path' \
                               THEN jsonb_array_length({j} -> 'relationships') END END)"
                        )
                    }
                }
            }
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
            // `coalesce(n.maybe_untyped, 0)` is ordinary Cypher, but SQL insists
            // every branch have one type — and an undeclared property reads out
            // as text. Pick the first branch whose type is known and read the
            // untyped ones at that type.
            "coalesce" if self.coalesce_as_jsonb(args, false) => self.coalesce_jsonb(args)?,
            "coalesce" => {
                let want = args.iter().find_map(|x| self.type_of(x));
                let branches: Vec<String> = match &want {
                    Some(t) if t != "text" => args
                        .iter()
                        .zip(&a)
                        .map(|(x, sql)| {
                            if self.type_of(x).is_none() {
                                format!("({sql})::{t}")
                            } else {
                                sql.clone()
                            }
                        })
                        .collect(),
                    _ => a.clone(),
                };
                format!("coalesce({})", branches.join(", "))
            }
            "ceil" => format!("ceil(({})::float8)", a[0]),
            "floor" => format!("floor(({})::float8)", a[0]),
            "round" => format!("round(({})::numeric)", a[0]),
            "sqrt" => format!("sqrt(({})::float8)", a[0]),
            "rand" => "random()".into(),
            "timestamp" => "extract(epoch from now())::int8".into(),
            "exists" => format!("({} IS NOT NULL)", a[0]),
            "keys" => format!("to_jsonb(ARRAY(SELECT jsonb_object_keys({})))", a[0]),
            // `properties(n)` is the map a node or relationship already
            // compiles to — the same value `keys()` reads. Neo4j has it and
            // drivers reach for it whenever they want the whole property bag
            // rather than a named one.
            //
            // The bookkeeping keys come off. In Neo4j this returns a *map*, and
            // leaving `_id`/`_type` in would both invent properties the caller
            // never set and make the driver hydrate the result as a node.
            "properties" => format!("(({}) - '_id' - '_type')", a[0]),
            // `x[i]` is two different operations wearing one syntax: list
            // indexing when `i` is a number, map lookup when it is a string.
            // Compiling it as `-> int` unconditionally made
            // `n[k] for k IN keys(n)` — the ordinary way to walk a node's
            // properties — fail with "cannot cast jsonb string to type
            // integer".
            //
            // Where the key's type is known at compile time, emit the operator
            // directly. Where it is not (a comprehension binder holds jsonb),
            // decide on the value's own json type at run time.
            "element_at" => {
                let obj = &a[0];
                let idx = &a[1];
                match self.type_of(&args[1]).as_deref() {
                    Some("int8") => format!("({obj} -> ({idx})::int)"),
                    Some("text") => format!("({obj} -> ({idx})::text)"),
                    _ => format!(
                        "(CASE WHEN jsonb_typeof(to_jsonb({idx})) = 'number' \
                         THEN {obj} -> ((to_jsonb({idx}) #>> '{{}}')::int) \
                         ELSE {obj} -> (to_jsonb({idx}) #>> '{{}}') END)"
                    ),
                }
            }
            // A path is `{"_path": true, "nodes": […], "relationships": […]}`.
            "nodes" => format!("(({}) -> 'nodes')", self.jsonb_of(&args[0])?),
            "relationships" | "rels" => {
                let j = self.jsonb_of(&args[0])?;
                // `[r*1..3]` binds the list of relationships itself.
                format!("(CASE WHEN jsonb_typeof({j}) = 'array' THEN {j} ELSE {j} -> 'relationships' END)")
            }
            "startnode" | "endnode" => {
                let end = if lname == "startnode" { "_src" } else { "_dst" };
                let r = self.jsonb_of(&args[0])?;
                format!("og_node_json((({r}) ->> '{end}')::int8)")
            }
            // Neo4j's randomUUID(): a version-4 UUID as a string.
            "randomuuid" => "gen_random_uuid()::text".into(),
            // spec 004 — vector surface
            "vector.similarity" | "similarity" => {
                format!("(1 - (({})::vector <=> ({})::vector))", a[0], a[1])
            }
            "vector.distance" => format!("(({})::vector <=> ({})::vector)", a[0], a[1]),
            // Neo4j 5's similarity functions, both normalised to [0, 1] as
            // Neo4j defines them: (1 + cos) / 2, and 1 / (1 + d²). A list
            // property or parameter reads as a vector through its text form.
            "vector.similarity.cosine" => {
                format!("((2 - (({})::text::vector <=> ({})::text::vector)) / 2)", a[0], a[1])
            }
            "vector.similarity.euclidean" => {
                format!("(1 / (1 + (({})::text::vector <-> ({})::text::vector) ^ 2))", a[0], a[1])
            }
            "vector.l2" => format!("(({})::vector <-> ({})::vector)", a[0], a[1]),
            // Neo4j's `genai.vector.encode(resource, provider, configuration)`.
            // Off by default, and the endpoint is configuration rather than an
            // argument — see `compat::genai`.
            "genai.vector.encode" => {
                if a.is_empty() {
                    return Err("genai.vector.encode(resource, provider, configuration) needs \
                                the text to encode"
                        .into());
                }
                let provider = a.get(1).cloned().unwrap_or_else(|| "NULL".into());
                let config = a.get(2).cloned().unwrap_or_else(|| "'{}'".into());
                format!(
                    "og_genai_encode(({})::text, ({provider})::text, ({config})::jsonb)",
                    a[0]
                )
            }
            other => {
                return Err(format!(
                    "unknown function '{other}'. supported: count, sum, avg, min, max, collect, \
                     head, last, tail, \
                     id, elementId, labels, type, length, size, toUpper, toLower, trim, substring, \
                     replace, split, coalesce, abs, ceil, floor, round, sqrt, rand, toString, \
                     toInteger, toFloat, timestamp, date, datetime, localdatetime, duration, exists, \
                     keys, vector.similarity, vector.distance, genai.vector.encode"
                ))
            }
        })
    }
}

/// Integer, exact or floating-point SQL number types.
fn is_numeric_type(t: &str) -> bool {
    matches!(t, "int2" | "int4" | "int8" | "numeric" | "float4" | "float8")
}

/// SQL types a jsonb scalar reads back as with `#>> '{}'` and a cast.
fn is_scalar_type(t: &str) -> bool {
    is_numeric_type(t)
        || matches!(t, "text" | "bool" | "date" | "timestamptz" | "timestamp" | "interval" | "uuid")
}

/// The variables and properties an aggregate expression reads outside its
/// aggregates — what the rows are grouped by.
fn bare_references(e: &Expr) -> Vec<Expr> {
    let mut out = Vec::new();
    fn walk(e: &Expr, out: &mut Vec<Expr>) {
        match e {
            Expr::Func { .. } if e.is_aggregate() && !matches!(e, Expr::Func { args, .. } if args.iter().any(|a| a.is_aggregate())) => {}
            Expr::Var(_) => out.push(e.clone()),
            Expr::Prop(b, _) if matches!(&**b, Expr::Var(_)) => out.push(e.clone()),
            Expr::Func { args, .. } => args.iter().for_each(|a| walk(a, out)),
            Expr::Binary(_, a, b) => {
                walk(a, out);
                walk(b, out);
            }
            Expr::Not(a) | Expr::Neg(a) | Expr::IsNull(a, _) | Expr::Prop(a, _) => walk(a, out),
            Expr::List(xs) => xs.iter().for_each(|x| walk(x, out)),
            Expr::Map(kv) => kv.iter().for_each(|(_, v)| walk(v, out)),
            Expr::Case { operand, whens, else_ } => {
                if let Some(o) = operand {
                    walk(o, out);
                }
                for (c, v) in whens {
                    walk(c, out);
                    walk(v, out);
                }
                if let Some(x) = else_ {
                    walk(x, out);
                }
            }
            Expr::Slice { list, from, to } => {
                walk(list, out);
                if let Some(f) = from {
                    walk(f, out);
                }
                if let Some(t) = to {
                    walk(t, out);
                }
            }
            _ => {}
        }
    }
    walk(e, &mut out);
    out
}

/// Is this, as written, a list? `+` concatenates lists.
fn is_list_expr(e: &Expr) -> bool {
    match e {
        Expr::List(_) | Expr::ListComp { .. } | Expr::Slice { .. } => true,
        Expr::Func { name, .. } => matches!(
            name.to_ascii_lowercase().as_str(),
            "collect" | "split" | "labels" | "keys" | "tail" | "nodes" | "relationships" | "range"
        ),
        _ => false,
    }
}

/// The type `a op b` has in SQL once an operand of unknown type is read as a
/// number (see `Compiler::arithmetic`).
fn arithmetic_type(op: BinOp, l: Option<&str>, r: Option<&str>) -> Option<String> {
    const TEMPORAL: &[&str] = &["date", "timestamptz", "timestamp", "interval"];
    let temporal = |t: Option<&str>| t.is_some_and(|t| TEMPORAL.contains(&t));
    if temporal(l) || temporal(r) {
        return match (op, l, r) {
            (BinOp::Add | BinOp::Sub, Some(t @ ("date" | "timestamptz" | "timestamp")), Some("interval")) => {
                Some(t.into())
            }
            (BinOp::Add, Some("interval"), Some(t @ ("date" | "timestamptz" | "timestamp"))) => Some(t.into()),
            (BinOp::Sub, Some("date"), Some("date")) => Some("int4".into()),
            (BinOp::Sub, Some(a), Some(b)) if a == b && a != "interval" => Some("interval".into()),
            (BinOp::Add | BinOp::Sub, Some("interval"), Some("interval")) => Some("interval".into()),
            _ => None,
        };
    }
    if op == BinOp::Add && (l == Some("text") || r == Some("text")) {
        return Some("text".into());
    }
    if op == BinOp::Mod {
        return Some("numeric".into());
    }
    let (kl, kr) = (l.filter(|t| is_numeric_type(t)), r.filter(|t| is_numeric_type(t)));
    match (kl, kr) {
        (Some(a), Some(b)) => {
            let float = |t: &str| t.starts_with("float");
            Some(if float(a) || float(b) {
                "float8"
            } else if a == "numeric" || b == "numeric" {
                "numeric"
            } else {
                "int8"
            }
            .into())
        }
        _ if l == Some("bool") || r == Some("bool") => None,
        _ if [l, r].iter().any(|t| t.is_some_and(|t| t != "text" && !is_numeric_type(t))) => None,
        // An operand of unknown type is read as numeric.
        (Some(a), None) | (None, Some(a)) if a.starts_with("float") => Some("float8".into()),
        _ => Some("numeric".into()),
    }
}

/// The relationships a variable-length hop walked (`og_vlp`'s `path`, edge ids
/// in order), as a jsonb list of relationships; null where an OPTIONAL MATCH
/// found no walk.
fn var_hop_rels(w: &str) -> String {
    format!(
        "(CASE WHEN {w}.path IS NULL THEN NULL ELSE (SELECT coalesce(jsonb_agg(og_edge_json(vx.e) \
         ORDER BY vx.o), '[]'::jsonb) FROM unnest({w}.path) WITH ORDINALITY AS vx(e, o)) END)"
    )
}

/// The nodes a variable-length hop from `start` reached, in order and ending at
/// its far end (the start itself not included): each edge's other end, walked
/// from the start — the only reading that is right for undirected hops too.
fn var_hop_nodes(start: &str, w: &str) -> String {
    format!(
        "(CASE WHEN {w}.path IS NULL THEN NULL ELSE (WITH RECURSIVE pn(i, node) AS (\
         SELECT 0, {start}.id UNION ALL \
         SELECT pn.i + 1, CASE WHEN pe.src = pn.node THEN pe.dst ELSE pe.src END \
           FROM pn JOIN og_data.og_edge pe ON pe.id = ({w}.path)[pn.i + 1] \
          WHERE pn.i < cardinality({w}.path)) \
         SELECT coalesce(jsonb_agg(og_node_json(pn.node) ORDER BY pn.i), '[]'::jsonb) \
           FROM pn WHERE pn.i > 0) END)"
    )
}

/// The key property of a virtual type: the property mapped from its key column.
fn virtual_key_property(tid: i32) -> Option<String> {
    crate::fabric::virtual_source(tid)?;
    crate::spiu::one::<String>(
        "SELECT p.key FROM og_catalog.mapping m, jsonb_each_text(m.property_map) p
          WHERE m.type_id = $1 AND p.value = m.id_column LIMIT 1",
        &[tid.into()],
    )
    .ok()
    .flatten()
}

/// A type others sit under: abstract, or with subtypes. `_Entity` in
/// Ontology Studio. See `Compiler::virtual_under_umbrella`.
fn is_umbrella(t: i32) -> bool {
    crate::spiu::one::<bool>(
        "SELECT t.is_abstract OR EXISTS (
             SELECT 1 FROM og_catalog.type_parent p WHERE p.parent_id = t.type_id)
           FROM og_catalog.type t WHERE t.type_id = $1",
        &[t.into()],
    )
    .ok()
    .flatten()
    .unwrap_or(false)
}

/// A predicate that is always false but names both ends of a hop, so the
/// OPTIONAL MATCH machinery places it on the hop's join (see `close_optional`).
fn never(a: &str, b: &str) -> String {
    format!("({a}.id IS NULL AND {b}.id IS NULL AND false)")
}

/// A string toInteger()/toFloat() reads as a number: optional sign, digits with
/// an optional fraction, optional exponent, surrounding blanks.
const NUMERIC_TEXT: &str = r"^\s*[-+]?(\d+\.?\d*|\.\d+)([eE][-+]?\d+)?\s*$";

/// The Cypher temporal field `p` names, lower-cased, if it names one.
fn temporal_unit(p: &str) -> Option<&'static str> {
    Some(match p.to_ascii_lowercase().as_str() {
        "year" => "year",
        "quarter" => "quarter",
        "month" => "month",
        "week" => "week",
        "day" => "day",
        "ordinalday" => "ordinalday",
        "dayofweek" | "weekday" => "dayofweek",
        "hour" => "hour",
        "minute" => "minute",
        "second" => "second",
        "millisecond" => "millisecond",
        "epochseconds" => "epochseconds",
        "epochmillis" => "epochmillis",
        _ => return None,
    })
}

/// `extract` for one temporal field, as the integer Cypher returns.
fn extract_sql(sql: &str, field: &str) -> String {
    match field {
        "ordinalday" => format!("extract(doy FROM {sql})::int8"),
        "dayofweek" => format!("extract(isodow FROM {sql})::int8"),
        "second" => format!("floor(extract(second FROM {sql}))::int8"),
        "millisecond" => format!("(floor(extract(milliseconds FROM {sql}))::int8 % 1000)"),
        "epochseconds" => format!("floor(extract(epoch FROM {sql}))::int8"),
        "epochmillis" => format!("floor(extract(epoch FROM {sql}) * 1000)::int8"),
        f => format!("extract({f} FROM {sql})::int8"),
    }
}

/// Does this SQL mention the alias as a whole word?
///
/// `n1` must not match inside `n10`, and aliases are always followed by `.` in
/// generated SQL, so the check is anchored on both sides.
fn mentions_alias(sql: &str, alias: &str) -> bool {
    let needle = format!("{alias}.");
    sql.match_indices(&needle).any(|(i, _)| {
        i == 0 || !sql.as_bytes()[i - 1].is_ascii_alphanumeric() && sql.as_bytes()[i - 1] != b'_'
    })
}

/// SQL identifier with proper quoting. Cypher names are case-sensitive and may
/// contain anything a backtick allows, so they are always quoted.
pub fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// SQL string literal with proper escaping.
pub fn sql_str(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The same pattern read from the other end: elements reversed, every
/// relationship's direction flipped.
fn reverse_elems(elems: &[PatElem]) -> Vec<PatElem> {
    elems
        .iter()
        .rev()
        .map(|e| match e {
            PatElem::Rel(r) => {
                let mut r = r.clone();
                r.dir = match r.dir {
                    Dir::Out => Dir::In,
                    Dir::In => Dir::Out,
                    Dir::Both => Dir::Both,
                };
                PatElem::Rel(r)
            }
            n => n.clone(),
        })
        .collect()
}
