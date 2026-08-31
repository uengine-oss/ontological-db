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

fn is_write(q: &Query) -> bool {
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
                | Clause::Ddl(_)
        )
    })
}

fn compile_cached(graph: &str, query: &str) -> Result<(String, Vec<String>), String> {
    let key = (graph.to_string(), query.to_string(), schema_epoch());
    if let Some(hit) = PLAN_CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return Ok(hit);
    }
    let ast = parser::parse(query)?;
    if is_write(&ast) {
        return Err("write queries are not compiled to a single statement".into());
    }
    let mut c = compile::Compiler::new(graph);
    let compiled = c.compile_read(&ast)?;
    let out = (compiled.sql, compiled.columns);
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
    let started = std::time::Instant::now();
    // Each call reports what *it* changed, so the count starts here rather than
    // accumulating over the connection. See `crate::stats`.
    crate::stats::reset();
    let ast = match parser::parse(query) {
        Ok(a) => a,
        Err(e) => {
            audit(graph, query, 0, started, Some(&e));
            error!("cypher parse error: {e}")
        }
    };

    let rows = if is_write(&ast) {
        run_write(graph, &ast, &params.0)
    } else {
        run_read(graph, query, &params.0)
    };

    audit(graph, query, rows.len() as i64, started, None);
    SetOfIterator::new(rows.into_iter().map(JsonB))
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
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    Spi::run_with_args(
        "INSERT INTO og_data.og_audit (query, lang, rows_out, duration_ms, error_code)
         VALUES ($1, 'cypher', $2, $3, $4)",
        &[
            format!("[{graph}] {query}").into(),
            rows.into(),
            ms.into(),
            err.map(|e| e.chars().take(200).collect::<String>()).into(),
        ],
    )
    .ok();
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
        table.filter_map(|r| r.get::<JsonB>(1).ok().flatten().map(|j| j.0)).collect()
    })
}

// --------------------------------------------------------------------------
// Write execution
// --------------------------------------------------------------------------

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

    // 1. Evaluate the read part (if any) to produce bindings.
    //
    // `WITH` belongs here. It used to stop the `take_while` and then fall
    // through the write loop's catch-all, which meant `MATCH (n) WITH n LIMIT 1
    // DELETE n` dropped the `LIMIT` and deleted the whole label. A clause that
    // changes which rows are written cannot be ignored — either it is honoured
    // or the query is refused.
    let read_clauses: Vec<Clause> = q
        .clauses
        .iter()
        .take_while(|c| {
            matches!(c, Clause::Match { .. } | Clause::Unwind { .. } | Clause::With { .. })
        })
        .cloned()
        .collect();

    let mut envs: Vec<eval::Env> = Vec::new();
    let mut with_proj: Option<Projection> = None;
    if read_clauses.is_empty() {
        envs.push(HashMap::new());
    } else {
        let mut c = compile::Compiler::new(graph);
        for clause in &read_clauses {
            match clause {
                Clause::Match { patterns, optional, where_ } => {
                    c.begin_match_clause();
                    for p in patterns {
                        if let Err(e) = c.compile_pattern(p, *optional) {
                            error!("cypher error: {e}");
                        }
                    }
                    if let Some(w) = where_ {
                        match c.expr(w, None) {
                            Ok(s) => c.push_where(s),
                            Err(e) => error!("cypher error: {e}"),
                        }
                    }
                }
                Clause::Unwind { expr, alias } => {
                    // `UNWIND $rows AS row MATCH (n) WHERE … SET …` is the
                    // standard way a Neo4j application writes a batch. The
                    // compiler has always been able to produce the rows; only
                    // this path refused them.
                    if let Err(e) = c.compile_unwind(expr, alias) {
                        error!("cypher error: {e}");
                    }
                }
                Clause::With { proj, where_ } => {
                    // What is honoured here is the part of `WITH` that decides
                    // *which rows* reach the write: DISTINCT, ORDER BY, SKIP,
                    // LIMIT, and a trailing WHERE. What is refused is the part
                    // that would re-shape the bindings — expressions, aliases
                    // and aggregates — because the write clauses that follow
                    // address pattern variables, and a projection that renames
                    // or computes them leaves nothing for `DELETE n` to mean.
                    //
                    // Refusing is the point. Every one of these used to be
                    // dropped on the floor along with the LIMIT beside it.
                    if with_proj.is_some() {
                        error!("only one WITH is supported before a write clause");
                    }
                    // 쓰기 절은 패턴 변수를 *이름으로* 지목한다. 따라서 위험한
                    // 것은 이름을 바꾸는 투영뿐이다. 계산된 항목은 새 이름으로
                    // env 에 실릴 뿐 기존 변수를 가리지 않으므로,
                    // `WITH bc, count(x) AS n MERGE …` 같은 형태는 허용한다.
                    if !proj.items.iter().all(|i| match &i.expr {
                        Expr::Var(v) => i.alias.as_deref().unwrap_or(v) == v,
                        _ => i.alias.is_some(),
                    }) {
                        error!(
                            "WITH before a write clause may not rename a variable, and every \
                             computed item needs an alias — the write clauses that follow \
                             address pattern variables by name"
                        );
                    }
                    if let Some(w) = where_ {
                        match c.expr(w, None) {
                            Ok(sql) => c.push_where(sql),
                            Err(e) => error!("cypher error: {e}"),
                        }
                    }
                    with_proj = Some(proj.clone());
                }
                _ => {}
            }
        }
        // A WITH that is not last would mean a second scoping horizon, and the
        // clauses after it were compiled against the first. Refuse rather than
        // compile something that means neither.
        if with_proj.is_some()
            && !matches!(read_clauses.last(), Some(Clause::With { .. }))
        {
            error!("WITH must be the last read clause before a write clause");
        }
        let proj = match &with_proj {
            Some(p) => p.clone(),
            None => Projection {
                items: c
                    .bound_vars()
                    .into_iter()
                    .map(|v| ReturnItem { expr: Expr::Var(v.clone()), alias: Some(v) })
                    .collect(),
                ..Default::default()
            },
        };
        if proj.items.is_empty() {
            envs.push(HashMap::new());
        } else {
            let compiled = match c.build_select_pub(&proj) {
                Ok(v) => v,
                Err(e) => error!("cypher error: {e}"),
            };
            for row in exec_json(&compiled.sql, params) {
                let mut env = HashMap::new();
                if let Value::Object(m) = row {
                    for (k, v) in m {
                        env.insert(k, v);
                    }
                }
                envs.push(env);
            }
        }
    }

    // 2. Apply write clauses per binding row.
    let write_clauses: Vec<&Clause> = q
        .clauses
        .iter()
        .skip(read_clauses.len())
        .collect();

    // `REMOVE n:Old SET n:New` — renaming a class. Recognised across the two
    // clauses and applied once to the catalog, before the per-row loop, because
    // it is a schema change and not a per-node one. See `rename_label`.
    let renamed = rename_label(gid, &write_clauses);

    let mut out = Vec::new();
    for env in envs.iter_mut() {
        for clause in &write_clauses {
            if renamed && matches!(clause, Clause::Set(items) | Clause::Remove(items)
                if items.iter().all(|i| matches!(i, SetOp::Label { .. })))
            {
                continue;
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
                Clause::Set(items) => apply_set(items, env, params),
                Clause::Remove(items) => apply_set(items, env, params),
                Clause::Delete { exprs, detach } => {
                    for e in exprs {
                        let v = eval::eval(e, env, params).unwrap_or(Value::Null);
                        if let Some(id) = v.get("_id").and_then(|x| x.as_i64()) {
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
                Clause::Return(proj) => {
                    let mut obj = Map::new();
                    for item in &proj.items {
                        let name =
                            item.alias.clone().unwrap_or_else(|| item.expr.default_alias());
                        // An aggregate cannot be evaluated one row at a time.
                        // Keep its *argument* per row; `fold_aggregates` folds
                        // the column once every row has been produced.
                        let value = match &item.expr {
                            Expr::Func { args, .. } if item.expr.is_aggregate() => {
                                match args.first() {
                                    None | Some(Expr::Star) => json!(true),
                                    Some(a) => eval::eval(a, env, params).unwrap_or(Value::Null),
                                }
                            }
                            e => eval::eval(e, env, params).unwrap_or(Value::Null),
                        };
                        obj.insert(name, value);
                    }
                    out.push(Value::Object(obj));
                }
                _ => {}
            }
        }
    }
    // `… RETURN count(n)` after a write is how an application asks how much it
    // changed. The write path walks bindings one row at a time and cannot see
    // across them, so the aggregate is folded here, over the rows it produced.
    if let Some(Clause::Return(proj)) = write_clauses.iter().find_map(|c| match c {
        Clause::Return(_) => Some(*c),
        _ => None,
    }) {
        if proj.items.iter().any(|i| i.expr.is_aggregate()) {
            return vec![fold_aggregates(proj, &out)];
        }
    }
    out
}

/// Fold a projection containing aggregates over the rows a write produced.
///
/// Only the aggregates Cypher's write path realistically ends with are folded —
/// counting and collecting what changed. A non-aggregate item takes its value
/// from the first row, which is what grouping by nothing means.
fn fold_aggregates(proj: &Projection, rows: &[Value]) -> Value {
    let mut obj = Map::new();
    for item in &proj.items {
        let name = item.alias.clone().unwrap_or_else(|| item.expr.default_alias());
        let value = match &item.expr {
            Expr::Func { name: f, args, distinct } => {
                let f = f.to_ascii_lowercase();
                let mut values: Vec<Value> = rows
                    .iter()
                    .filter_map(|r| r.get(&name).cloned())
                    .filter(|v| !v.is_null())
                    .collect();
                if *distinct {
                    // `Vec::dedup` removes *consecutive* duplicates, and these
                    // rows arrive in whatever order the plan produced — so
                    // `count(DISTINCT x)` counted runs, not values. serde_json's
                    // map is a BTreeMap unless `preserve_order` is on, so a
                    // value's serialisation is canonical and usable as identity;
                    // `Value` implements neither `Hash` nor `Ord`, which is why
                    // this goes through the text form rather than a set of
                    // values. Order of first appearance is preserved, which is
                    // what `collect(DISTINCT x)` should return.
                    let mut seen = std::collections::HashSet::new();
                    values.retain(|v| seen.insert(v.to_string()));
                }
                match f.as_str() {
                    // `count(*)` counts rows; `count(x)` counts non-null x. The
                    // per-row pass already evaluated x into this column.
                    "count" if matches!(args.first(), Some(Expr::Star)) => json!(rows.len()),
                    "count" => json!(values.len()),
                    "collect" => Value::Array(values),
                    "sum" | "avg" => {
                        let nums: Vec<f64> =
                            values.iter().filter_map(serde_json::Value::as_f64).collect();
                        let total: f64 = nums.iter().sum();
                        if f == "sum" {
                            json!(total)
                        } else if nums.is_empty() {
                            Value::Null
                        } else {
                            json!(total / nums.len() as f64)
                        }
                    }
                    "min" => values.into_iter().min_by(cmp_json).unwrap_or(Value::Null),
                    "max" => values.into_iter().max_by(cmp_json).unwrap_or(Value::Null),
                    _ => Value::Null,
                }
            }
            _ => rows.first().and_then(|r| r.get(&name).cloned()).unwrap_or(Value::Null),
        };
        obj.insert(name, value);
    }
    Value::Object(obj)
}

fn cmp_json(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
        _ => a.to_string().cmp(&b.to_string()),
    }
}

fn create_pattern(graph: &str, gid: i32, p: &Pattern, env: &mut eval::Env, params: &Value) {
    let mut prev: Option<i64> = None;
    let mut pending: Option<&RelPat> = None;

    for elem in &p.elems {
        match elem {
            PatElem::Node(np) => {
                let id = match np.var.as_ref().and_then(|v| env.get(v)) {
                    Some(v) if v.get("_id").is_some() => v["_id"].as_i64().unwrap(),
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
    // Try to find it first, with the pattern's inline properties as the key.
    let mut c = compile::Compiler::new(graph);
    if c.compile_pattern(p, false).is_ok() {
        let vars = c.bound_vars();
        // Variables an earlier clause already bound are fixed points, not free
        // ones to match again: `MERGE (a)-[:DIRECTED]->(m)` must look for an
        // edge between *these* two nodes. Without pinning, the pattern matches
        // any DIRECTED edge in the graph and MERGE never creates a second one.
        for v in &vars {
            let Some(id) = env.get(v).and_then(|e| e.get("_id")).and_then(Value::as_i64) else {
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
        let proj = Projection {
            items: vars
                .iter()
                .map(|v| ReturnItem { expr: Expr::Var(v.clone()), alias: Some(v.clone()) })
                .collect(),
            limit: Some(Expr::Lit(Lit::Int(1))),
            ..Default::default()
        };
        if !proj.items.is_empty() {
            if let Ok(compiled) = c.build_select_pub(&proj) {
                let rows = exec_json(&compiled.sql, params);
                if let Some(Value::Object(m)) = rows.into_iter().next() {
                    for (k, v) in m {
                        env.insert(k, v);
                    }
                    apply_set(on_match, env, params);
                    return;
                }
            }
        }
    }
    create_pattern(graph, gid, p, env, params);
    apply_set(on_create, env, params);
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

fn apply_set(items: &[SetOp], env: &mut eval::Env, params: &Value) {
    for item in items {
        match item {
            SetOp::Prop { var, prop, value } => {
                let Some(entity) = env.get(var).cloned() else {
                    error!("SET refers to unbound variable '{var}'");
                };
                let Some(id) = entity.get("_id").and_then(|x| x.as_i64()) else { continue };
                let v = eval::eval(value, env, params).unwrap_or(Value::Null);
                let mut m = Map::new();
                m.insert(prop.clone(), v.clone());
                write_props(id, entity.get("_src").is_some(), Value::Object(m));
                if let Some(Value::Object(e)) = env.get_mut(var) {
                    e.insert(prop.clone(), v);
                }
            }
            SetOp::Map { var, value, merge } => {
                let Some(entity) = env.get(var).cloned() else {
                    error!("SET refers to unbound variable '{var}'");
                };
                let Some(id) = entity.get("_id").and_then(|x| x.as_i64()) else { continue };
                let v = eval::eval(value, env, params).unwrap_or(Value::Null);
                if !*merge {
                    // `SET a = {..}` replaces: clear existing extension payload.
                    write_props(id, entity.get("_src").is_some(), json!({}));
                }
                write_props(id, entity.get("_src").is_some(), v);
            }
            SetOp::Label { var, labels } => {
                // Adding a label the node already carries — its own type or one
                // above it — is what `SET n:Label` usually means in code that
                // re-tags on every write. That is a no-op here, not an error.
                let Some(entity) = env.get(var).cloned() else {
                    error!("SET/REMOVE refers to unbound variable '{var}'");
                };
                let Some(id) = entity.get("_id").and_then(|x| x.as_i64()) else { continue };
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
        m.insert(k.clone(), eval::eval(v, env, params).unwrap_or(Value::Null));
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
    let plan = crate::spiu::one_mut::<JsonB>(
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
