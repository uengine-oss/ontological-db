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
        self.binds.insert(alias.to_string(), Bind::Scalar { sql: format!("{a}.v") });
        Ok(())
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
    fn compile_with(&mut self, proj: &Projection, where_: Option<&Expr>) -> CResult<()> {
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
    fn compile_call(
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
                "{with}SELECT jsonb_build_object({}) AS row FROM (\n{inner}\n) t{tail}",
                json_pairs.join(", ")
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
        if optional {
            self.notes.push("OPTIONAL MATCH compiled as LEFT JOIN LATERAL".into());
        }
        let mut prev_node: Option<String> = None;
        let mut pending_rel: Option<&RelPat> = None;
        let mut hop_exprs: Vec<String> = Vec::new();

        // `MATCH p = (a)-[*1..3]->(b)` binds the paths themselves, so this
        // pattern must enumerate them however multiplicity-blind the RETURN is.
        let outer_reachability = self.reachability_only;
        if p.path_var.is_some() {
            self.reachability_only = false;
        }

        for elem in &p.elems {
            match elem {
                PatElem::Node(np) => {
                    let mark = self.from.len();
                    let alias = self.bind_node(np, optional)?;
                    let node_join_added = self.from.len() > mark;
                    if let (Some(prev), Some(rel)) = (prev_node.clone(), pending_rel.take()) {
                        let hop = self.join_rel(&prev, rel, &alias, optional)?;
                        hop_exprs.push(hop);
                        // The node is constrained by the hop that reaches it, so
                        // its join has to come *after* the hop's — otherwise the
                        // predicate cannot be an ON condition and the node joins
                        // to everything. Only matters under OPTIONAL MATCH,
                        // where predicates live in ON rather than WHERE.
                        if node_join_added && mark > 0 && self.from.len() > mark + 1 {
                            self.move_join_to_end(mark);
                        }
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
        self.reachability_only = outer_reachability;
        Ok(())
    }

    /// Resolve a label set, reporting separately whether it can match at all.
    /// Callers decide what "cannot match" means where they stand: an empty
    /// result for a plain MATCH, a row of NULLs under OPTIONAL MATCH.
    fn resolve_label_match(&mut self, labels: &[String]) -> CResult<(Option<i32>, bool)> {
        match types::resolve_label_set(self.gid, &self.graph, labels)? {
            types::LabelMatch::Any => Ok((None, true)),
            types::LabelMatch::Type(t) => Ok((Some(t), true)),
            types::LabelMatch::Nothing => Ok((None, false)),
        }
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
            None => "og_data.og_node".to_string(),
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
            self.constrain(format!("{lhs} = {rhs}"));
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

        if let Some((min, max)) = rel.range {
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
                let ejoin = if optional { "LEFT JOIN" } else { "JOIN" };
                    self.from.push(format!("{ejoin} {ev} {ea} ON {ea}.id = {u}.eid"));
                    self.note_optional_join(&ea);
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
            self.constrain(format!("{lhs} = {rhs}"));
        }
        Ok(())
    }

    // ----------------------------------------------------------------------
    // Expressions
    // ----------------------------------------------------------------------

    /// SQL for a property access, plus the column's SQL type when known.
    pub fn prop_sql(&self, alias: &str, tid: Option<i32>, prop: &str) -> (String, Option<String>) {
        // `_id` is the internal identifier. `id` is NOT: Neo4j exposes the
        // internal one as `id(n)` and treats `n.id` as an ordinary user
        // property, so short-circuiting it here silently shadows a stored
        // `id` — the read returns the int8 while the write kept the string.
        if prop == "_id" {
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
            let props = views::view_properties(t);
            if let Some((col, _)) = props.get(prop) {
                return format!("to_jsonb({alias}.{col})");
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
        if let Expr::Prop(base, prop) = e {
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
                let props = views::view_properties(t);
                let mut pairs = vec![
                    format!("'_id', {alias}.id"),
                    format!("'_type', og_type_name({alias}.type_id)"),
                ];
                for (name, (col, _)) in &props {
                    pairs.push(format!("{}, to_jsonb({alias}.{col})", sql_str(name)));
                }
                // OPTIONAL MATCH 가 매치하지 못하면 조인 컬럼이 전부 NULL 이 된다.
                // 그대로 두면 jsonb_strip_nulls 가 `{{}}` 로 접어서 SQL NULL 이
                // 아니게 되고, `x IS NULL` 과 `count(x)` 가 틀린 답을 낸다.
                format!(
                    "(CASE WHEN {alias}.id IS NULL THEN NULL ELSE \
                       (jsonb_strip_nulls(jsonb_build_object({})) || COALESCE({alias}.__ext,'{{}}'::jsonb)) END)",
                    pairs.join(", ")
                )
            }
            None => format!(
                "(CASE WHEN {alias}.id IS NULL THEN NULL ELSE og_node_json({alias}.id) END)"
            ),
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
        // node_json 과 같은 이유 — 미매치 관계는 `{{}}` 가 아니라 NULL 이어야 한다.
        format!(
            "(CASE WHEN {alias}.id IS NULL THEN NULL ELSE \
               (jsonb_strip_nulls(jsonb_build_object({})) || COALESCE({alias}.__ext,'{{}}'::jsonb)) END)",
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
            // `[x IN xs WHERE p | e]` — a scalar subquery over the unnested
            // list. The binder is an ordinary scalar binding while the body
            // compiles, then goes out of scope, so a comprehension cannot leak
            // a variable into the enclosing query.
            Expr::ListComp { var, source, filter, project } => {
                let src = self.expr(source, None)?;
                let a = self.fresh("lc");
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
                let (proj, cond) = body?;
                format!(
                    "(SELECT coalesce(jsonb_agg({proj}), '[]'::jsonb) \
                       FROM jsonb_array_elements(({src})::jsonb) AS {a}(v){cond})"
                )
            }
            Expr::ListPred { kind, var, source, filter } => {
                let src = self.expr(source, None)?;
                let a = self.fresh("lp");
                let shadowed = self
                    .binds
                    .insert(var.clone(), Bind::Scalar { sql: format!("{a}.v") });
                let cond = self.expr(filter, Some("bool"));
                self.restore_bind(var, shadowed);
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
                    format!("jsonb_build_object({})", pairs.join(", "))
                };
                match base {
                    // Explicit entries win over `.*`, as in Cypher.
                    Some(b) => format!("({b} || {built})"),
                    None => built,
                }
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
        // `x IN xs` needs the right-hand side as jsonb whatever it came from —
        // a parameter arrives as text and containment is a jsonb operator.
        if op == BinOp::In {
            let ls = self.expr(l, None)?;
            let rs = self.expr(r, Some("jsonb"))?;
            return Ok(format!("(({rs}) @> to_jsonb({ls}))"));
        }

        let lhint = self.type_of(r);
        let rhint = self.type_of(l);
        let mut ls = self.expr(l, lhint.as_deref())?;
        let mut rs = self.expr(r, rhint.as_deref())?;

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
            BinOp::In => unreachable!("handled above"),
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
            // `coalesce(n.maybe_untyped, 0)` is ordinary Cypher, but SQL insists
            // every branch have one type — and an undeclared property reads out
            // as text. Pick the first branch whose type is known and read the
            // untyped ones at that type.
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
                     id, elementId, labels, type, length, size, toUpper, toLower, trim, substring, \
                     replace, split, coalesce, abs, ceil, floor, round, sqrt, rand, toString, \
                     toInteger, toFloat, timestamp, datetime, exists, keys, vector.similarity, \
                     vector.distance, genai.vector.encode"
                ))
            }
        })
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
