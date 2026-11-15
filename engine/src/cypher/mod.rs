//! Cypher engine — spec 003.
//!
//! Pipeline: lex → parse → AST → compile to SQL → PostgreSQL executes.
//!
//! Read queries become a single SQL statement, so the planner owns join order,
//! scan selection and parallelism. Write queries run procedurally over the
//! bindings the read part produced, inside the caller's transaction.

pub mod ast;
pub mod compile;
pub mod eval;
pub mod lexer;
pub mod parser;
pub mod timeout;
pub mod views;

use ast::*;
use pgrx::prelude::*;
use pgrx::JsonB;
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::collections::HashMap;

/// Upper bound applied when `*..` is written without an explicit maximum.
pub const MAX_VAR_LENGTH: u32 = 8;

thread_local! {
    /// Compiled-SQL cache. Parsing and compiling is the expensive part of a
    /// repeated query; PostgreSQL caches the resulting plan itself.
    static PLAN_CACHE: RefCell<HashMap<(String, String, i64), (String, Vec<String>)>> =
        RefCell::new(HashMap::new());
}

/// The schema counter, as a cache-key component.
///
/// The cache used to be keyed on `(graph, query)` alone, so a compiled plan
/// outlived the schema it was compiled against. The visible failure is worse
/// than a stale view name: when a property is promoted out of `__ext` into its
/// own column, a cached plan keeps reading `__ext ->> 'x'` while new writes go
/// to `p_x`, and the query returns rows with the value silently missing.
/// Property promotion happens on an ordinary write, so this needs no DDL to
/// trigger.
///
/// Clearing the cache from `bump_schema_version` would only fix the backend
/// that ran the DDL — the cache is thread-local, one per backend — so the
/// version belongs in the key. Reading it costs one lookup of a sequence's
/// `last_value`, which is O(1) and does not grow with the number of schema
/// changes the way `max(version)` over the table would.
fn schema_epoch() -> i64 {
    crate::spiu::one::<i64>("SELECT last_value FROM og_catalog.schema_version_seq", &[])
        .ok()
        .flatten()
        .unwrap_or(0)
}

pub(crate) fn is_write(q: &Query) -> bool {
    if let Some((_, tail)) = &q.union {
        if is_write(tail) {
            return true;
        }
    }
    q.clauses.iter().any(|c| {
        matches!(
            c,
            Clause::Create(_)
                | Clause::Merge { .. }
                | Clause::Set(_)
                | Clause::Remove(_)
                | Clause::Delete { .. }
                | Clause::Foreach { .. }
                | Clause::Ddl(_)
        ) || matches!(c, Clause::Subquery { body, .. } if is_write(body))
    })
}

pub(crate) fn compile_cached(graph: &str, query: &str) -> Result<(String, Vec<String>), String> {
    let key = (graph.to_string(), query.to_string(), schema_epoch());
    if let Some(hit) = PLAN_CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return Ok(hit);
    }
    let ast = parser::parse(query)?;
    if is_write(&ast) {
        return Err("write queries are not compiled to a single statement".into());
    }
    views::take_created_view();
    let mut c = compile::Compiler::new(graph);
    let compiled = c.compile_read(&ast)?;
    let out = (compiled.sql, compiled.columns);
    // A view this compile created exists only once its transaction commits; a
    // rollback drops it, and a cached plan would then name a missing relation
    // ("relation og_data.v_N does not exist") for the rest of the session.
    // Compile again next time — by then the view either exists or is rebuilt.
    if views::take_created_view() {
        return Ok(out);
    }
    PLAN_CACHE.with(|cache| {
        let mut m = cache.borrow_mut();
        if m.len() > 512 {
            m.clear();
        }
        m.insert(key, out.clone());
    });
    Ok(out)
}

/// Show the SQL a Cypher query compiles to.
///
/// This is the honest form of "the optimiser can see your pattern": paste the
/// output into `EXPLAIN` — or into your own SQL — and it behaves like any other
/// query, because it is one.
#[pg_extern(stable)]
fn og_cypher_sql(graph: &str, query: &str) -> String {
    if crate::fabric::sqlish::is_statement(query) {
        return crate::fabric::sqlish::shown_sql(query, graph).unwrap_or_else(|e| error!("fabric statement: {e}"));
    }
    match compile_cached(graph, query) {
        Ok((sql, _)) => sql,
        Err(e) => error!("{e}"),
    }
}

/// Execute a Cypher query. Returns one jsonb object per result row.
#[pg_extern]
fn og_cypher(
    graph: &str,
    query: &str,
    params: default!(JsonB, "'{}'"),
) -> SetOfIterator<'static, JsonB> {
    let rows = run_query(graph, query, &params.0);
    SetOfIterator::new(rows.into_iter().map(JsonB))
}

/// The body of `og_cypher`, callable from Rust — actions run their Cypher
/// through here so that it is parsed, checked and audited the same way.
pub fn run_query(graph: &str, query: &str, params: &Value) -> Vec<Value> {
    let started = std::time::Instant::now();
    // Every statement gets a deadline (`og.statement_timeout`, two minutes
    // unless set): a pattern over virtual nodes is a query on a source system,
    // and a cartesian product of two of them otherwise runs for hours.
    timeout::arm();
    // Each call reports what *it* changed, so the count starts here rather than
    // accumulating over the connection. See `crate::stats`.
    crate::stats::reset();
    // A fabric statement (CREATE DATASOURCE, SHOW TABLES, CREATE VIRTUAL NODE …)
    // is not Cypher, but it arrives wherever Cypher does — the Studio, a Bolt
    // driver — so this is where it is recognised and handed on.
    if crate::fabric::sqlish::is_statement(query) {
        let p = match params {
            Value::Object(m) => m.clone(),
            _ => serde_json::Map::new(),
        };
        return crate::fabric::sqlish::run(query, Some(graph), &p);
    }
    let ast = match parser::parse(query) {
        Ok(a) => a,
        Err(e) => {
            audit(graph, query, 0, started, Some(&e));
            error!("cypher parse error: {e}")
        }
    };

    let rows = if is_write(&ast) {
        run_write(graph, &ast, params)
    } else {
        run_read(graph, query, params)
    };

    audit(graph, query, rows.len() as i64, started, None);
    rows
}

/// What the last `og_cypher()` on this connection changed.
///
/// Volatile and connection-scoped by construction: it reads state the previous
/// call left behind, so it must be asked on the same connection and before the
/// next query. The Bolt gateway asks it between finishing a write and writing
/// the summary, which is the only window in which the answer means anything.
#[pg_extern(volatile, parallel_unsafe)]
fn og_cypher_stats() -> JsonB {
    JsonB(crate::stats::snapshot())
}

fn audit(graph: &str, query: &str, rows: i64, started: std::time::Instant, err: Option<&str>) {
    crate::security::audit("cypher", graph, query, rows, started, err);
}

fn run_read(graph: &str, query: &str, params: &Value) -> Vec<Value> {
    let (sql, _cols) = match compile_cached(graph, query) {
        Ok(v) => v,
        Err(e) => error!("cypher error: {e}"),
    };
    exec_json(&sql, params)
}

fn exec_json(sql: &str, params: &Value) -> Vec<Value> {
    Spi::connect(|client| {
        let table = client
            .select(sql, None, &[JsonB(params.clone()).into()])
            .unwrap_or_else(|e| error!("cypher execution failed: {e}\n--- compiled SQL ---\n{sql}"));
        // Reading the rows out is Rust's loop, not the executor's, so nothing in
        // it looked at a pending cancel: a timeout or a cancel request that
        // arrived after SPI finished waited until every row had been converted
        // (seconds per million rows).
        table
            .filter_map(|r| {
                pgrx::check_for_interrupts!();
                r.get::<JsonB>(1).ok().flatten().map(|j| j.0)
            })
            .collect()
    })
}

// --------------------------------------------------------------------------
// Write execution
// --------------------------------------------------------------------------

/// Clauses that only read: they turn rows of bindings into rows of bindings.
fn is_read_clause(c: &Clause) -> bool {
    matches!(c, Clause::Match { .. } | Clause::Unwind { .. } | Clause::With { .. } | Clause::Call { .. })
        || matches!(c, Clause::Subquery { body, .. } if !is_write(body))
}

/// The parameter a read segment after a write receives the rows so far in.
const ROWS_PARAM: &str = "__og_rows";

fn run_write(graph: &str, q: &Query, params: &Value) -> Vec<Value> {
    // Schema commands stand alone — Cypher does not let them share a statement
    // with a pattern, so they are handled before any binding work.
    if let Some(Clause::Ddl(stmt)) = q.clauses.first() {
        if q.clauses.len() > 1 {
            error!("a schema command cannot be combined with other clauses");
        }
        return crate::compat::ddl::run(graph, stmt, params);
    }

    if q.union.is_some() {
        error!("UNION cannot be combined with a write clause");
    }

    let gid = crate::catalog::types::graph_id(graph);

    // `REMOVE n:Old SET n:New` — renaming a class. Recognised across the two
    // clauses and applied once to the catalog — after the rows are matched
    // (by the old name), before the first one is written — because it is a
    // schema change and not a per-node one. See `rename_label`.
    let write_clauses: Vec<&Clause> =
        q.clauses.iter().filter(|c| !is_read_clause(c) && !matches!(c, Clause::Return(_))).collect();
    let mut renamed: Option<bool> = None;

    // The query runs as a pipeline of rows, clause by clause, as Cypher reads.
    //
    // A run of reading clauses (MATCH, UNWIND, WITH, CALL … YIELD, a CALL { … }
    // that only reads) is compiled to one SQL statement and turns the rows so
    // far into new rows; a writing clause is applied to each row. The reading
    // part used to be allowed only in front: `WITH` before a write could only
    // pass plain variables through, and a reading clause *after* a write —
    // `MERGE (a:A {k: 1}) WITH a MATCH (b:B {k: 2}) MERGE (a)-[:R]->(b)`, the
    // ordinary way to link two nodes — was skipped without a word, so `b` was a
    // new, empty node. Now each run of reading clauses after the first starts
    // from the rows written so far.
    let mut rename = Rename { clauses: write_clauses, done: renamed.take() };
    run_clauses(graph, gid, &q.clauses, None, params, Some(&mut rename)).map(|(rows, _)| rows).unwrap_or_default()
}

/// The one rename a top-level `REMOVE n:Old SET n:New` stands for, decided at
/// the first write.
struct Rename<'q> {
    clauses: Vec<&'q Clause>,
    done: Option<bool>,
}

/// Run `clauses` over rows: from the start of a query (`start` = None), or from
/// `start`'s rows and variables (a subquery's body, one outer row at a time).
/// Returns the rows of a final RETURN and their column names, or None when the
/// clauses end without one.
fn run_clauses(
    graph: &str,
    gid: i32,
    clauses: &[Clause],
    start: Option<(Vec<eval::Env>, Vec<String>)>,
    params: &Value,
    mut rename: Option<&mut Rename>,
) -> Option<(Vec<Value>, Vec<String>)> {
    let seeded = start.is_some();
    let (mut envs, mut names) = start.unwrap_or_else(|| (vec![HashMap::new()], Vec::new()));
    let mut i = 0;
    while i < clauses.len() {
        if is_read_clause(&clauses[i]) {
            let end = clauses[i..].iter().position(|c| !is_read_clause(c)).map_or(clauses.len(), |k| i + k);
            let seed = if i == 0 && !seeded { None } else { Some((envs.as_slice(), names.as_slice())) };
            let (rows, bound) = read_segment(graph, &clauses[i..end], None, seed, params);
            names = bound;
            envs = rows
                .into_iter()
                .map(|row| match row {
                    Value::Object(m) => {
                        let mut env: eval::Env = m.into_iter().collect();
                        env.remove(ROW_MARK);
                        env
                    }
                    _ => HashMap::new(),
                })
                .collect();
            i = end;
            continue;
        }
        if let Clause::Return(proj) = &clauses[i] {
            // The result is projected by SQL over the rows the writes produced,
            // so RETURN after a write has everything RETURN has elsewhere —
            // ORDER BY, SKIP, LIMIT, DISTINCT, aggregates, every function.
            let (rows, _) = read_segment(graph, &[], Some(proj), Some((&envs, &names)), params);
            let cols = proj.items.iter().map(|it| it.alias.clone().unwrap_or_else(|| it.expr.default_alias())).collect();
            return Some((rows, cols));
        }
        if let Clause::Subquery { scope, body, optional } = &clauses[i] {
            let (e, n) = run_subquery(graph, gid, scope.as_deref(), body, *optional, envs, names, params);
            envs = e;
            names = n;
            i += 1;
            continue;
        }
        let renamed = match rename.as_deref_mut() {
            Some(r) => *r.done.get_or_insert_with(|| rename_label(gid, &r.clauses)),
            None => false,
        };
        for env in envs.iter_mut() {
            // One binding's writes are a handful of SPI calls; a MATCH that bound a
            // million rows makes this loop long, and a cancel has to reach it.
            pgrx::check_for_interrupts!();
            apply_write(graph, gid, &clauses[i], env, params, renamed);
        }
        let introduced: Vec<&Pattern> = match &clauses[i] {
            Clause::Create(pats) => pats.iter().collect(),
            Clause::Merge { pattern, .. } => vec![pattern],
            _ => Vec::new(),
        };
        for p in introduced {
            names.extend(p.path_var.iter().cloned());
            for e in &p.elems {
                let v = match e {
                    PatElem::Node(n) => &n.var,
                    PatElem::Rel(r) => &r.var,
                };
                names.extend(v.iter().cloned());
            }
        }
        names.sort();
        names.dedup();
        i += 1;
    }
    None
}

/// `[OPTIONAL] CALL { … }` whose body writes: the body runs once per row, in
/// order, each run seeing what the runs before it wrote — Cypher's semantics,
/// which a single SQL statement cannot have. The body starts from the row's
/// imported variables only. Each row the body RETURNs joins its outer row (no
/// rows: the outer row is dropped, or kept with nulls under OPTIONAL); a body
/// without RETURN (a "unit" subquery) leaves the rows as they were.
#[allow(clippy::too_many_arguments)]
fn run_subquery(
    graph: &str,
    gid: i32,
    scope: Option<&[String]>,
    body: &Query,
    optional: bool,
    envs: Vec<eval::Env>,
    names: Vec<String>,
    params: &Value,
) -> (Vec<eval::Env>, Vec<String>) {
    let mut branches: Vec<(&Query, bool)> = vec![(body, true)];
    let mut q = body;
    while let Some((all, tail)) = &q.union {
        branches.push((tail, *all));
        q = tail;
    }
    let distinct = branches.iter().skip(1).any(|(_, all)| !*all);
    let mut out = Vec::new();
    let mut returned: Option<Vec<String>> = None;
    for env in envs {
        pgrx::check_for_interrupts!();
        let mut rows: Vec<Value> = Vec::new();
        let mut any_return = false;
        let mut imported_all: Vec<String> = Vec::new();
        for (branch, _) in &branches {
            let (imports, clauses) =
                compile::subquery_imports(scope, branch, &names).unwrap_or_else(|e| error!("cypher error: {e}"));
            for v in &imports {
                if !names.contains(v) {
                    error!("cypher error: CALL {{ … }} imports `{v}`, which is not defined before it");
                }
            }
            let seed: eval::Env = env.iter().filter(|(k, _)| imports.contains(k)).map(|(k, v)| (k.clone(), v.clone())).collect();
            if let Some((rs, cols)) = run_clauses(graph, gid, clauses, Some((vec![seed], imports.clone())), params, None) {
                any_return = true;
                if returned.is_none() {
                    returned = Some(cols);
                }
                rows.extend(rs);
            }
            imported_all.extend(imports);
        }
        // The outer row's elements as the body left them.
        let mut env = env;
        for k in &imported_all {
            let fresh = env.get(k).and_then(|v| {
                let id = v.get("_id").and_then(Value::as_i64)?;
                let sql = if v.get("_src").is_some() { "SELECT og_edge_json($1)" } else { "SELECT og_node_json($1)" };
                crate::spiu::one::<JsonB>(sql, &[id.into()]).ok().flatten().map(|j| j.0)
            });
            if let Some(f) = fresh {
                env.insert(k.clone(), f);
            }
        }
        if !any_return {
            out.push(env);
            continue;
        }
        if distinct {
            let mut seen = std::collections::HashSet::new();
            rows.retain(|r| seen.insert(r.to_string()));
        }
        if rows.is_empty() {
            if optional {
                let mut e = env.clone();
                for c in returned.iter().flatten() {
                    e.entry(c.clone()).or_insert(Value::Null);
                }
                out.push(e);
            }
            continue;
        }
        for r in rows {
            let mut e = env.clone();
            if let Value::Object(m) = r {
                for (k, v) in m {
                    if k != ROW_MARK && !imported_all.contains(&k) {
                        if names.contains(&k) {
                            error!("cypher error: the subquery returns `{k}`, which is already defined outside it — return it under another name");
                        }
                        e.insert(k, v);
                    }
                }
            }
            out.push(e);
        }
    }
    let mut names = names;
    names.extend(returned.unwrap_or_default());
    names.sort();
    names.dedup();
    (out, names)
}

/// A column every read segment projects, so that a segment binding no
/// variables still yields one row per match (`MATCH (:A) CREATE (:B)` creates
/// a B for every A — it used to create one).
const ROW_MARK: &str = "__og_row";

/// Compile and run a run of reading clauses, starting from `seed` (the rows a
/// write produced, and the variables in scope) when given, and project either
/// every bound variable or `ret`. Returns one object per row, and the
/// variables bound at the end.
fn read_segment(
    graph: &str,
    clauses: &[Clause],
    ret: Option<&Projection>,
    seed: Option<(&[eval::Env], &[String])>,
    params: &Value,
) -> (Vec<Value>, Vec<String>) {
    let mut c = compile::Compiler::new(graph);
    let mut p = params.clone();
    if let Some((rows, known)) = seed {
        let mut names: Vec<String> = rows.iter().flat_map(|e| e.keys().cloned()).collect();
        names.extend(known.iter().cloned());
        names.sort();
        names.dedup();
        // A variable that is a stored node of the same type in every row can
        // anchor a pattern on that type's view.
        let mut types = HashMap::new();
        for n in &names {
            let mut tid: Option<i32> = None;
            let mut same = true;
            for e in rows {
                let t = e
                    .get(n)
                    .filter(|v| v.get("_src").is_none())
                    .and_then(|v| v.get("_id"))
                    .and_then(Value::as_i64)
                    .map(crate::id::id_type);
                match (t, tid) {
                    (Some(t), None) => tid = Some(t),
                    (Some(t), Some(have)) if t == have => {}
                    _ => {
                        same = false;
                        break;
                    }
                }
            }
            if let (true, Some(t)) = (same, tid) {
                types.insert(n.clone(), t);
            }
        }
        let list: Vec<Value> =
            rows.iter().map(|e| Value::Object(e.iter().map(|(k, v)| (k.clone(), v.clone())).collect())).collect();
        if let Value::Object(m) = &mut p {
            m.insert(ROWS_PARAM.into(), Value::Array(list));
        } else {
            p = json!({ ROWS_PARAM: list });
        }
        let flags: Vec<String> = names
            .iter()
            .filter(|n| {
                let mut any = false;
                rows.iter().all(|e| match e.get(*n) {
                    Some(Value::Bool(_)) => {
                        any = true;
                        true
                    }
                    Some(Value::Null) | None => true,
                    _ => false,
                }) && any
            })
            .cloned()
            .collect();
        c.seed_rows(ROWS_PARAM, &names, &types, &flags);
    }
    for clause in clauses {
        let r = match clause {
            Clause::Match { patterns, optional, where_ } => c.compile_match(patterns, *optional, where_.as_ref()),
            Clause::Unwind { expr, alias } => c.compile_unwind(expr, alias),
            Clause::With { proj, where_ } => c.compile_with(proj, where_.as_ref()),
            Clause::Call { name, args, yields } => c.compile_call(name, args, yields),
            Clause::Subquery { scope, body, optional } => c.compile_subquery(scope.as_deref(), body, *optional),
            _ => Err("this clause cannot be part of a read".into()),
        };
        if let Err(e) = r {
            error!("cypher error: {e}");
        }
    }
    let proj = match ret {
        Some(p) => p.clone(),
        None => {
            let mut items: Vec<ReturnItem> = c
                .bound_vars()
                .into_iter()
                .map(|v| ReturnItem { expr: Expr::Var(v.clone()), alias: Some(v) })
                .collect();
            items.push(ReturnItem { expr: Expr::Lit(Lit::Int(1)), alias: Some(ROW_MARK.into()) });
            Projection { items, ..Default::default() }
        }
    };
    let bound = c.bound_vars();
    let compiled = c.build_select_pub(&proj).unwrap_or_else(|e| error!("cypher error: {e}"));
    (exec_json(&compiled.sql, &p), bound)
}

/// Apply one writing clause to one row.
fn apply_write(graph: &str, gid: i32, clause: &Clause, env: &mut eval::Env, params: &Value, renamed: bool) {
    if renamed
        && matches!(clause, Clause::Set(items) | Clause::Remove(items)
            if items.iter().all(|i| matches!(i, SetOp::Label { .. })))
    {
        return;
    }
    match clause {
        Clause::Create(pats) => {
            for p in pats {
                create_pattern(graph, gid, p, env, params);
            }
        }
        Clause::Merge { pattern, on_create, on_match } => {
            merge_pattern(graph, gid, pattern, on_create, on_match, env, params);
        }
        Clause::Set(items) => apply_set(gid, items, env, params),
        Clause::Remove(items) => apply_set(gid, items, env, params),
        Clause::Delete { exprs, detach } => {
            for e in exprs {
                let v = eval_or_fail(e, env, params);
                if let Some(id) = element_id(gid, &v, "DELETE") {
                    let is_edge = v.get("_src").is_some();
                    if is_edge {
                        crate::storage::delete_edge_inner(id);
                    } else {
                        if !*detach {
                            let deg = crate::spiu::one::<i64>(
                                "SELECT COALESCE(sum(n),0)::int8 FROM og_data.og_adj WHERE src = $1",
                                &[id.into()],
                            )
                            .ok()
                            .flatten()
                            .unwrap_or(0);
                            if deg > 0 {
                                error!(
                                    "cannot delete node {id}: it still has {deg} \
                                     relationship(s). use DETACH DELETE"
                                );
                            }
                        }
                        crate::storage::delete_node_inner(id);
                    }
                }
            }
        }
        // `FOREACH (x IN list | <writes>)`: the writes once per element, with
        // `x` bound. Variables the body introduces stay inside it, as in
        // Cypher; what it does to the row's own variables (SET n.x) stays.
        Clause::Foreach { var, list, body } => {
            let items = match eval_or_fail(list, env, params) {
                Value::Array(xs) => xs,
                Value::Null => Vec::new(),
                other => error!("FOREACH needs a list, got {other}"),
            };
            for item in items {
                let mut local = env.clone();
                local.insert(var.clone(), item);
                for c in body {
                    apply_write(graph, gid, c, &mut local, params, renamed);
                }
                for (k, v) in local {
                    if k != *var && env.contains_key(&k) {
                        env.insert(k, v);
                    }
                }
            }
        }
        other => error!("this clause cannot be used here: {}", clause_name(other)),
    }
}

fn clause_name(c: &Clause) -> &'static str {
    match c {
        Clause::Match { .. } => "MATCH",
        Clause::Unwind { .. } => "UNWIND",
        Clause::With { .. } => "WITH",
        Clause::Return(_) => "RETURN",
        Clause::Create(_) => "CREATE",
        Clause::Merge { .. } => "MERGE",
        Clause::Set(_) => "SET",
        Clause::Remove(_) => "REMOVE",
        Clause::Delete { .. } => "DELETE",
        Clause::Call { .. } => "CALL",
        Clause::Foreach { .. } => "FOREACH",
        Clause::Subquery { .. } => "CALL { … }",
        Clause::Ddl(_) => "a schema command",
    }
}

/// A value for a write. An expression the write path cannot evaluate used to
/// become null without a word — `SET n.id = randomUUID()` made the node and
/// left the property empty — so it is an error now.
fn eval_or_fail(e: &Expr, env: &eval::Env, params: &Value) -> Value {
    eval::eval(e, env, params).unwrap_or_else(|err| error!("cypher error: {err}"))
}

fn create_pattern(graph: &str, gid: i32, p: &Pattern, env: &mut eval::Env, params: &Value) {
    let mut prev: Option<i64> = None;
    let mut pending: Option<&RelPat> = None;

    for elem in &p.elems {
        match elem {
            PatElem::Node(np) => {
                let id = match np.var.as_ref().and_then(|v| env.get(v)) {
                    Some(v) if v.get("_id").is_some() => {
                        element_id(gid, v, "CREATE").unwrap_or_else(|| error!("CREATE: '{}' is not a node", np.var.as_deref().unwrap_or("")))
                    }
                    _ => {
                        let tid = crate::catalog::types::resolve_or_create_label_set(
                            gid,
                            graph,
                            &np.labels,
                            "entity",
                        );
                        let label = np.labels.last().map(String::as_str).unwrap_or("");
                        let props = props_json(&np.props, env, params);
                        let id = crate::storage::create_node_inner(gid, tid, label, props);
                        if let Some(v) = &np.var {
                            env.insert(v.clone(), node_json(id));
                        }
                        id
                    }
                };
                if let (Some(src), Some(rel)) = (prev, pending.take()) {
                    create_rel(graph, gid, rel, src, id, env, params);
                }
                prev = Some(id);
            }
            PatElem::Rel(rp) => pending = Some(rp),
        }
    }
}

fn create_rel(graph: &str, gid: i32, rel: &RelPat, a: i64, b: i64, env: &mut eval::Env, params: &Value) {
    if rel.types.len() != 1 {
        error!("CREATE requires exactly one relationship type");
    }
    let (src, dst) = match rel.dir {
        Dir::In => (b, a),
        _ => (a, b),
    };
    // Same rule as node labels: a relationship type written for the first time
    // is declared, not refused.
    let tid = crate::catalog::types::resolve_or_create_label_set(
        gid,
        graph,
        &rel.types[..1],
        "relation",
    );
    let props = props_json(&rel.props, env, params);
    let eid = crate::storage::create_edge_inner(gid, tid, &rel.types[0], src, dst, props);
    if let Some(v) = &rel.var {
        env.insert(v.clone(), edge_json(eid));
    }
}

fn merge_pattern(
    graph: &str,
    gid: i32,
    p: &Pattern,
    on_create: &[SetOp],
    on_match: &[SetOp],
    env: &mut eval::Env,
    params: &Value,
) {
    // The pattern's inline properties are the key it is looked up by. Their
    // values are fixed here, from the row, and handed to the lookup as
    // parameters: the lookup is compiled on its own, where a row variable —
    // `UNWIND $rows AS item MERGE (n:L {product_id: item.node_id})` — is not
    // bound. It used to fail to compile there, and the failure fell through to
    // CREATE: every MERGE under UNWIND made a new node (or broke a unique
    // constraint). Evaluated once, the same values serve the lookup and the
    // create.
    let mut pat = p.clone();
    let mut bound = match params {
        Value::Object(m) => m.clone(),
        _ => Map::new(),
    };
    let mut k = 0usize;
    for elem in pat.elems.iter_mut() {
        let (props, what) = match elem {
            PatElem::Node(n) => (&mut n.props, "node"),
            PatElem::Rel(r) => (&mut r.props, "relationship"),
        };
        for (key, e) in props.iter_mut() {
            let v = eval_or_fail(e, env, params);
            if v.is_null() {
                // Neo4j refuses this too: a null key matches nothing, so every
                // run would create another.
                error!(
                    "cannot MERGE a {what} on a null property value for '{key}' — \
                     leave the rows without it out first (WHERE … IS NOT NULL)"
                );
            }
            let name = format!("__og_merge_{k}");
            k += 1;
            bound.insert(name.clone(), v);
            *e = Expr::Param(name);
        }
    }
    let params = &Value::Object(bound);

    // Try to find it first.
    let mut c = compile::Compiler::new(graph);
    if let Err(e) = c.compile_pattern(&pat, false) {
        error!("cypher error: {e}");
    }
    let vars = c.bound_vars();
    // Variables an earlier clause already bound are fixed points, not free
    // ones to match again: `MERGE (a)-[:DIRECTED]->(m)` must look for an
    // edge between *these* two nodes. Without pinning, the pattern matches
    // any DIRECTED edge in the graph and MERGE never creates a second one.
    for v in &vars {
        let Some(id) = env.get(v).and_then(|e| element_id(gid, e, "MERGE")) else {
            continue;
        };
        match c.binding(v) {
            Some(compile::Bind::Node { alias, .. })
            | Some(compile::Bind::Rel { alias: Some(alias), .. }) => {
                let w = format!("{alias}.id = {id}");
                c.push_where(w);
            }
            _ => {}
        }
    }
    let mut items: Vec<ReturnItem> = vars
        .iter()
        .map(|v| ReturnItem { expr: Expr::Var(v.clone()), alias: Some(v.clone()) })
        .collect();
    // A pattern with no variable (`MERGE (:Tag {name: 'x'})`) still has to be
    // looked up — it used to be created every time.
    items.push(ReturnItem { expr: Expr::Lit(Lit::Int(1)), alias: Some(ROW_MARK.into()) });
    let proj = Projection { items, limit: Some(Expr::Lit(Lit::Int(1))), ..Default::default() };
    let compiled = c.build_select_pub(&proj).unwrap_or_else(|e| error!("cypher error: {e}"));
    if let Some(Value::Object(m)) = exec_json(&compiled.sql, params).into_iter().next() {
        for (k, v) in m {
            if k != ROW_MARK {
                env.insert(k, v);
            }
        }
        apply_set(gid, on_match, env, params);
        return;
    }
    create_pattern(graph, gid, &pat, env, params);
    apply_set(gid, on_create, env, params);
}

/// Recognise `REMOVE n:Old SET n:New` and carry it out as a rename.
///
/// A node's type is encoded in its identifier here, so a node cannot change
/// type without changing identity — and changing identity would invalidate
/// every adjacency entry pointing at it. Renaming the *type* achieves what this
/// query means, for every instance at once, in constant time and with all
/// identifiers intact.
///
/// The narrow shape is deliberate: it applies only when the removed label is a
/// real type and the added one is free. Anything else is a genuine retype and is
/// refused by `apply_set` with that said plainly.
fn rename_label(gid: i32, clauses: &[&Clause]) -> bool {
    let mut removed: Option<(&str, &str)> = None; // (var, label)
    let mut added: Option<(&str, &str)> = None;

    for clause in clauses {
        let (items, is_remove) = match clause {
            Clause::Remove(items) => (items, true),
            Clause::Set(items) => (items, false),
            _ => continue,
        };
        for item in items {
            if let SetOp::Label { var, labels } = item {
                if labels.len() != 1 {
                    return false;
                }
                let slot = if is_remove { &mut removed } else { &mut added };
                if slot.is_some() {
                    return false;
                }
                *slot = Some((var.as_str(), labels[0].as_str()));
            }
        }
    }

    let (Some((rv, old)), Some((av, new))) = (removed, added) else { return false };
    if rv != av {
        return false;
    }
    let Some(old_id) = crate::catalog::types::try_type_id(gid, old) else {
        // Already renamed — a repeat of the same statement, which must not fail.
        return crate::catalog::types::try_type_id(gid, new).is_some();
    };
    if crate::catalog::types::try_type_id(gid, new).is_some() {
        // Both exist: this is a move between types, not a rename.
        return false;
    }

    Spi::run_with_args(
        "UPDATE og_catalog.type SET name = $2 WHERE type_id = $1",
        &[old_id.into(), new.into()],
    )
    .unwrap_or_else(|e| error!("failed to rename label '{old}' to '{new}': {e}"));
    // The human-readable view is named after the type, so it moves with it.
    crate::catalog::types::drop_alias_view(old);
    if let Some(table) = crate::catalog::types::storage_table(old_id) {
        crate::catalog::types::ensure_alias_view(old_id, new, &table);
    }
    // Named indexes point at the label by name. Leaving them behind would keep
    // the name resolvable while what it names no longer exists.
    Spi::run_with_args(
        "UPDATE og_catalog.compat_index SET type_name = $3
          WHERE graph_id = $1 AND type_name = $2",
        &[gid.into(), old.into(), new.into()],
    )
    .unwrap_or_else(|e| error!("failed to move indexes from '{old}' to '{new}': {e}"));
    crate::catalog::labeling::bump_schema_version(gid, &format!("rename {old} -> {new}"));
    true
}

/// The identifier of the element a row's value stands for — when it is one of
/// this graph's.
///
/// A write takes its target from the row: a node a MATCH bound, or anything
/// else carrying an `_id`. A map parameter carries one as easily
/// (`WITH $m AS n SET n.x = 1`, `$m = {_id: …}`), and such a map used to reach
/// whatever node that number named — in another graph (project) as readily as
/// in this one: SET wrote to it, CREATE hung a relationship off it, DETACH
/// DELETE removed it. Reads were already confined to the graph; writes are now
/// too. A map that is not an element at all (`{_id: 'x'}`) is no target.
fn element_id(gid: i32, v: &Value, clause: &str) -> Option<i64> {
    let id = v.get("_id").and_then(Value::as_i64)?;
    let tid = crate::id::id_type(id);
    if type_graph(tid) != Some(gid) {
        error!(
            "{clause}: {id} is not a node or relationship of this graph — a map with an `_id` \
             key does not stand for an element; MATCH the element instead"
        );
    }
    Some(id)
}

thread_local! {
    /// Type → graph. A type never moves between graphs, so this cannot go stale.
    static TYPE_GRAPH: std::cell::RefCell<HashMap<i32, i32>> = std::cell::RefCell::new(HashMap::new());
}

fn type_graph(tid: i32) -> Option<i32> {
    if let Some(g) = TYPE_GRAPH.with(|m| m.borrow().get(&tid).copied()) {
        return Some(g);
    }
    let g = crate::spiu::one::<i32>("SELECT graph_id FROM og_catalog.type WHERE type_id = $1", &[tid.into()])
        .ok()
        .flatten()?;
    TYPE_GRAPH.with(|m| m.borrow_mut().insert(tid, g));
    Some(g)
}

fn apply_set(gid: i32, items: &[SetOp], env: &mut eval::Env, params: &Value) {
    for item in items {
        match item {
            SetOp::Prop { var, prop, value } => {
                let Some(entity) = env.get(var).cloned() else {
                    error!("SET refers to unbound variable '{var}'");
                };
                let Some(id) = element_id(gid, &entity, "SET") else { continue };
                let v = eval_or_fail(value, env, params);
                let mut m = Map::new();
                m.insert(prop.clone(), v.clone());
                write_props(id, entity.get("_src").is_some(), Value::Object(m));
                if let Some(Value::Object(e)) = env.get_mut(var) {
                    // A property set to null is gone, as in Cypher.
                    if v.is_null() {
                        e.remove(prop);
                    } else {
                        e.insert(prop.clone(), v);
                    }
                }
            }
            SetOp::Map { var, value, merge } => {
                let Some(entity) = env.get(var).cloned() else {
                    error!("SET refers to unbound variable '{var}'");
                };
                let Some(id) = element_id(gid, &entity, "SET") else { continue };
                let v = eval_or_fail(value, env, params);
                let is_edge = entity.get("_src").is_some();
                if !*merge {
                    // `SET a = {..}` replaces: clear existing extension payload.
                    write_props(id, is_edge, json!({}));
                }
                write_props(id, is_edge, v);
                // The row carries the element as it is now, for what follows
                // (a RETURN, a later clause).
                env.insert(var.clone(), if is_edge { edge_json(id) } else { node_json(id) });
            }
            SetOp::Label { var, labels } => {
                // Adding a label the node already carries — its own type or one
                // above it — is what `SET n:Label` usually means in code that
                // re-tags on every write. That is a no-op here, not an error.
                let Some(entity) = env.get(var).cloned() else {
                    error!("SET/REMOVE refers to unbound variable '{var}'");
                };
                let Some(id) = element_id(gid, &entity, "SET") else { continue };
                let tid = crate::id::id_type(id);
                let gid = crate::spiu::one::<i32>(
                    "SELECT graph_id FROM og_catalog.type WHERE type_id = $1",
                    &[tid.into()],
                )
                .ok()
                .flatten()
                .unwrap_or(0);

                for l in labels {
                    let already = crate::catalog::types::try_type_id(gid, l)
                        .is_some_and(|want| crate::catalog::labeling::og_is_subtype(tid, want));
                    if already {
                        continue;
                    }
                    error!(
                        "cannot add label '{l}' to this node: a node's type is part of its \
                         identifier here, so it cannot gain one after creation. To rename a \
                         class, write `REMOVE n:Old SET n:New` — that is applied to the type. \
                         To move a node between types, create it under the target type."
                    );
                }
            }
        }
    }
}

fn write_props(id: i64, is_edge: bool, props: Value) {
    if is_edge {
        crate::storage::set_edge_props_inner(id, props);
    } else {
        crate::storage::set_node_props_inner(id, props);
    }
}

fn props_json(props: &[(String, Expr)], env: &eval::Env, params: &Value) -> Value {
    let mut m = Map::new();
    for (k, v) in props {
        m.insert(k.clone(), eval_or_fail(v, env, params));
    }
    Value::Object(m)
}

fn node_json(id: i64) -> Value {
    crate::spiu::one::<JsonB>("SELECT og_node_json($1)", &[id.into()])
        .ok()
        .flatten()
        .map(|j| j.0)
        .unwrap_or(json!({ "_id": id }))
}

fn edge_json(id: i64) -> Value {
    crate::spiu::one::<JsonB>("SELECT og_edge_json($1)", &[id.into()])
        .ok()
        .flatten()
        .map(|j| j.0)
        .unwrap_or(json!({ "_id": id }))
}

// --------------------------------------------------------------------------
// Diagnostics
// --------------------------------------------------------------------------

/// `EXPLAIN` for a Cypher query — spec 003 FR-017.
#[pg_extern]
fn og_cypher_explain(graph: &str, query: &str, analyze: default!(bool, false)) -> JsonB {
    let (sql, cols) = match compile_cached(graph, query) {
        Ok(v) => v,
        Err(e) => error!("{e}"),
    };
    let opts = if analyze { "ANALYZE, BUFFERS, FORMAT JSON" } else { "FORMAT JSON" };
    // EXPLAIN's JSON format is `json`, not `jsonb` — read as jsonb it fails,
    // and the `.ok()` below turned that into a plan of null.
    let plan = crate::spiu::one_mut::<pgrx::Json>(
        &format!("EXPLAIN ({opts}) {sql}"),
        &[JsonB(json!({})).into()],
    )
    .ok()
    .flatten()
    .map(|j| j.0)
    .unwrap_or(Value::Null);
    JsonB(json!({
        "columns": cols,
        "sql": sql,
        "plan": plan,
    }))
}

/// Parse only — used by the portal and by agents to validate before executing.
#[pg_extern(immutable, parallel_safe)]
fn og_cypher_check(query: &str) -> JsonB {
    if crate::fabric::sqlish::is_statement(query) {
        return JsonB(json!({
            "ok": true,
            "clauses": 1,
            "write": crate::fabric::sqlish::is_write(query),
            "fabric": true,
        }));
    }
    match parser::parse(query) {
        Ok(q) => JsonB(json!({
            "ok": true,
            "clauses": q.clauses.len(),
            "write": is_write(&q),
        })),
        Err(e) => JsonB(json!({ "ok": false, "error": e })),
    }
}

/// The result column names, in `RETURN` order.
///
/// `og_cypher()` returns jsonb objects, and jsonb sorts its keys — so the row
/// itself cannot tell you the order the query asked for. The Bolt gateway
/// (spec 011) has to announce `fields` before the first record, so it asks the
/// parser instead of guessing. Parse only; no database access.
#[pg_extern(immutable, parallel_safe)]
fn og_cypher_columns(query: &str) -> Vec<String> {
    if crate::fabric::sqlish::is_statement(query) {
        return crate::fabric::sqlish::columns(query);
    }
    let q = match parser::parse(query) {
        Ok(q) => q,
        Err(e) => error!("{e}"),
    };
    let Some(Clause::Return(proj)) =
        q.clauses.iter().rev().find(|c| matches!(c, Clause::Return(_)))
    else {
        return Vec::new();
    };
    // `RETURN *` expands to the variables the patterns bound, which the parser
    // alone cannot know. Report nothing rather than a wrong order; the caller
    // falls back to the keys of the first row.
    if proj.items.iter().any(|it| matches!(it.expr, Expr::Star)) {
        return Vec::new();
    }
    proj.items
        .iter()
        .map(|it| it.alias.clone().unwrap_or_else(|| it.expr.default_alias()))
        .collect()
}

/// Compile without executing — used by the diagnostics surface (spec 008).
pub fn compile_for_diagnostics(graph: &str, query: &str) -> Result<String, String> {
    compile_cached(graph, query).map(|(sql, _)| sql)
}

/// Walk a MATCH pattern element by element, counting rows after each addition.
/// The first zero is where the match died (spec 008 FR-010).
pub fn diagnose_pattern(graph: &str, q: &Query) -> Vec<Value> {
    let mut steps = Vec::new();
    let Some(Clause::Match { patterns, where_, .. }) =
        q.clauses.iter().find(|c| matches!(c, Clause::Match { .. }))
    else {
        return steps;
    };

    for pattern in patterns {
        let mut upto = Vec::new();
        for elem in &pattern.elems {
            upto.push(elem.clone());
            // A pattern must end on a node to be well formed.
            if matches!(elem, PatElem::Rel(_)) {
                continue;
            }
            let partial = Pattern { path_var: None, elems: upto.clone() };
            let mut c = compile::Compiler::new(graph);
            if c.compile_pattern(&partial, false).is_err() {
                continue;
            }
            let proj = Projection {
                items: vec![ReturnItem {
                    expr: Expr::Func { name: "count".into(), args: vec![Expr::Star], distinct: false },
                    alias: Some("n".into()),
                }],
                ..Default::default()
            };
            let Ok(compiled) = c.build_select_pub(&proj) else { continue };
            let n = exec_json(&compiled.sql, &json!({}))
                .first()
                .and_then(|v| v.get("n").and_then(|x| x.as_i64()))
                .unwrap_or(-1);
            steps.push(json!({
                "elements": upto.len(),
                "description": describe(&upto),
                "rows": n,
            }));
            if n == 0 {
                steps.push(json!({
                    "verdict": "this is where the match became empty",
                    "hint": "check the label spelling, the relationship type, and the direction \
                             of the arrow at this step",
                }));
                return steps;
            }
        }
    }

    if where_.is_some() {
        steps.push(json!({
            "verdict": "the pattern matched; the WHERE clause removed every row",
            "hint": "relax the predicate or check property names with og_schema()",
        }));
    }
    steps
}

fn describe(elems: &[PatElem]) -> String {
    elems
        .iter()
        .map(|e| match e {
            PatElem::Node(n) => {
                let l = if n.labels.is_empty() { String::new() } else { format!(":{}", n.labels.join(":")) };
                format!("({}{})", n.var.clone().unwrap_or_default(), l)
            }
            PatElem::Rel(r) => {
                let t = if r.types.is_empty() { String::new() } else { format!(":{}", r.types.join("|")) };
                match r.dir {
                    Dir::Out => format!("-[{t}]->"),
                    Dir::In => format!("<-[{t}]-"),
                    Dir::Both => format!("-[{t}]-"),
                }
            }
        })
        .collect()
}
