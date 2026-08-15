//! TypeQL query surface — spec 010.
//!
//! A second first-class query language over the *same* catalog, storage and
//! transaction as Cypher. Reads compile to one SQL statement; writes run
//! procedurally per binding row.
//!
//! The pipeline is built by wrapping: each shaping stage becomes a subquery
//! around the previous one, so `sort` before `limit` and `limit` before `sort`
//! produce genuinely different SQL rather than being normalised into one shape
//! (FR-036).

pub mod ast;
pub mod compile;
pub mod dump;
pub mod lexer;
pub mod parser;
pub mod schema;
pub mod write;

use ast::*;
use compile::Compiler;
use pgrx::prelude::*;
use pgrx::JsonB;
use serde_json::{json, Value};
use write::{Env, EnvVal};

type QResult<T> = Result<T, String>;

fn is_write(q: &Query) -> bool {
    q.stages.iter().any(|s| {
        matches!(
            s,
            Stage::Insert(_)
                | Stage::Put(_)
                | Stage::Delete(_)
                | Stage::Update(_)
                | Stage::Define(_)
                | Stage::Undefine(_)
        )
    })
}

// --------------------------------------------------------------------------
// SQL surface
// --------------------------------------------------------------------------

/// Execute a TypeQL query. Returns one jsonb object per result row.
#[pg_extern]
fn og_typeql(
    graph: &str,
    query: &str,
    _params: default!(JsonB, "'{}'"),
) -> SetOfIterator<'static, JsonB> {
    let started = std::time::Instant::now();
    let gid = crate::catalog::types::graph_id(graph);
    let rows = match run_script(gid, query) {
        Ok(r) => r,
        Err(e) => {
            audit(graph, query, 0, started, Some(&e));
            error!("typeql error: {e}");
        }
    };
    audit(graph, query, rows.len() as i64, started, None);
    SetOfIterator::new(rows.into_iter().map(JsonB))
}

/// Parse and validate without executing — spec 010 FR-002.
#[pg_extern(immutable, parallel_safe)]
fn og_typeql_check(query: &str) -> JsonB {
    match parser::parse_script(query) {
        Ok(qs) => JsonB(json!({
            "ok": true,
            "queries": qs.len(),
            "write": qs.iter().any(is_write),
        })),
        Err(e) => JsonB(json!({ "ok": false, "error": e })),
    }
}

/// Show the SQL a read query compiles to — the same transparency the Cypher
/// surface offers (FR-003).
#[pg_extern(stable)]
fn og_typeql_sql(graph: &str, query: &str) -> String {
    let gid = crate::catalog::types::graph_id(graph);
    let q = match parser::parse(query) {
        Ok(q) => q,
        Err(e) => error!("typeql parse error: {e}"),
    };
    if is_write(&q) {
        error!("only read queries compile to a single SQL statement");
    }
    match compile_read(gid, &q) {
        Ok(sql) => sql,
        Err(e) => error!("typeql error: {e}"),
    }
}

/// Run a `.tql` script: several queries, executed in order, in one transaction.
#[pg_extern]
fn og_typeql_script(graph: &str, script: &str) -> i64 {
    let gid = crate::catalog::types::graph_id(graph);
    let queries = match parser::parse_script(script) {
        Ok(q) => q,
        Err(e) => error!("typeql parse error: {e}"),
    };
    let total = queries.len() as i64;
    for (i, q) in queries.iter().enumerate() {
        if let Err(e) = run_query(gid, q) {
            error!("typeql error in block {} of {}: {e}", i + 1, total);
        }
    }
    total
}

fn audit(graph: &str, query: &str, rows: i64, started: std::time::Instant, err: Option<&str>) {
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    Spi::run_with_args(
        "INSERT INTO og_data.og_audit (query, lang, rows_out, duration_ms, error_code)
         VALUES ($1, 'typeql', $2, $3, $4)",
        &[
            format!("[{graph}] {query}").into(),
            rows.into(),
            ms.into(),
            err.map(|e| e.chars().take(200).collect::<String>()).into(),
        ],
    )
    .ok();
}

// --------------------------------------------------------------------------
// Execution
// --------------------------------------------------------------------------

fn run_script(gid: i32, src: &str) -> QResult<Vec<Value>> {
    let queries = parser::parse_script(src)?;
    let mut out = Vec::new();
    for q in &queries {
        out = run_query(gid, q)?;
    }
    Ok(out)
}

fn run_query(gid: i32, q: &Query) -> QResult<Vec<Value>> {
    // Schema stages stand alone.
    if let Some(Stage::Define(defs)) = q.stages.first() {
        schema::run_define(gid, defs)?;
        if q.stages.len() == 1 {
            return Ok(vec![json!({"defined": defs.len()})]);
        }
    }
    if q.stages.iter().any(|s| matches!(s, Stage::Undefine(_))) {
        return Err("'undefine' is not implemented yet (spec 010 phase 5)".into());
    }

    if is_write(q) {
        run_write_pipeline(gid, q)
    } else {
        let sql = compile_read(gid, q)?;
        Ok(exec(&sql))
    }
}

// --------------------------------------------------------------------------
// Read pipeline
// --------------------------------------------------------------------------

/// Every bound variable becomes two columns: its value and, where it has one,
/// its identity. Shaping stages then wrap this query without ever needing to
/// look back at the original aliases.
struct Projected {
    sql: String,
    /// (var, value column, id column present)
    cols: Vec<(String, bool)>,
    types: std::collections::HashMap<String, i32>,
    attrs: Vec<String>,
}

fn vcol(v: &str) -> String {
    write::quote_ident(&format!("v_{v}"))
}

fn icol(v: &str) -> String {
    write::quote_ident(&format!("i_{v}"))
}

fn compile_match(gid: i32, q: &Query) -> QResult<Projected> {
    let mut pats: Vec<ast::Pat> = Vec::new();
    for s in &q.stages {
        if let Stage::Match(p) = s {
            pats.extend(p.iter().cloned());
        }
    }
    if pats.is_empty() {
        return Err("a read query needs a 'match' stage".into());
    }

    // Groups that share no variable also share no join decision, so each is
    // compiled on its own and the results are cross joined.
    let groups = compile::components(&pats);
    let mut parts = Vec::new();
    let mut cols = Vec::new();
    let mut attrs = Vec::new();
    let mut types = std::collections::HashMap::new();
    let mut outer_sel = Vec::new();

    for (i, g) in groups.iter().enumerate() {
        let mut c = Compiler::new(gid);
        c.compile_patterns(g)?;
        let vars = c.bound_vars();
        if vars.is_empty() {
            continue;
        }
        let alias = format!("g{i}");
        let mut sel = Vec::new();
        for v in &vars {
            sel.push(format!("{} AS {}", c.var_value(v)?, vcol(v)));
            outer_sel.push(format!("{alias}.{} AS {}", vcol(v), vcol(v)));
            let has_id = match c.var_id(v) {
                Ok(idsql) => {
                    sel.push(format!("{idsql} AS {}", icol(v)));
                    outer_sel.push(format!("{alias}.{} AS {}", icol(v), icol(v)));
                    true
                }
                Err(_) => false,
            };
            cols.push((v.clone(), has_id));
            if c.is_attr(v) {
                attrs.push(v.clone());
            }
        }
        types.extend(c.declared_types());
        parts.push(format!(
            "(SELECT {}{}{}) {alias}",
            sel.join(", "),
            c.from_sql(),
            c.where_sql()
        ));
    }

    if cols.is_empty() {
        return Err("the 'match' stage binds no variables".into());
    }
    let sql = format!("SELECT {} FROM {}", outer_sel.join(", "), parts.join(" CROSS JOIN "));
    Ok(Projected { sql, cols, types, attrs })
}

fn compile_read(gid: i32, q: &Query) -> QResult<String> {
    let mut p = compile_match(gid, q)?;
    let mut depth = 0usize;

    for stage in &q.stages {
        match stage {
            Stage::Match(_) | Stage::Define(_) | Stage::Undefine(_) => {}
            Stage::Select(vars) => {
                for v in vars {
                    if !p.cols.iter().any(|(n, _)| n == v) {
                        return Err(format!("'select ${v}': that variable is not bound"));
                    }
                }
                let kept: Vec<(String, bool)> =
                    p.cols.iter().filter(|(n, _)| vars.contains(n)).cloned().collect();
                let mut sel = Vec::new();
                for (v, has_id) in &kept {
                    sel.push(vcol(v));
                    if *has_id {
                        sel.push(icol(v));
                    }
                }
                depth += 1;
                p.sql = format!("SELECT {} FROM ({}) s{depth}", sel.join(", "), p.sql);
                p.cols = kept;
            }
            Stage::Distinct => {
                depth += 1;
                p.sql = format!("SELECT DISTINCT * FROM ({}) s{depth}", p.sql);
            }
            Stage::Sort(keys) => {
                let mut order = Vec::new();
                for (v, asc) in keys {
                    if !p.cols.iter().any(|(n, _)| n == v) {
                        return Err(format!("'sort ${v}': that variable is not bound"));
                    }
                    order.push(format!("{} {}", vcol(v), if *asc { "ASC" } else { "DESC" }));
                }
                depth += 1;
                p.sql =
                    format!("SELECT * FROM ({}) s{depth} ORDER BY {}", p.sql, order.join(", "));
            }
            Stage::Limit(n) => {
                depth += 1;
                p.sql = format!("SELECT * FROM ({}) s{depth} LIMIT {n}", p.sql);
            }
            Stage::Offset(n) => {
                depth += 1;
                p.sql = format!("SELECT * FROM ({}) s{depth} OFFSET {n}", p.sql);
            }
            Stage::Reduce { assigns, groupby } => {
                let mut sel = Vec::new();
                let mut cols = Vec::new();
                for g in groupby {
                    if !p.cols.iter().any(|(n, _)| n == g) {
                        return Err(format!("'groupby ${g}': that variable is not bound"));
                    }
                    let has_id = p.cols.iter().find(|(n, _)| n == g).map(|(_, i)| *i).unwrap();
                    sel.push(vcol(g));
                    if has_id {
                        sel.push(icol(g));
                    }
                    cols.push((g.clone(), has_id));
                }
                for r in assigns {
                    sel.push(format!("{} AS {}", agg_sql(r)?, vcol(&r.var)));
                    cols.push((r.var.clone(), false));
                }
                // The identity column travels with its value column, so it has
                // to be grouped too — it is functionally dependent, but only
                // the catalog knows that, not the planner.
                let mut keys: Vec<String> = Vec::new();
                for g in groupby {
                    keys.push(vcol(g));
                    if cols.iter().any(|(n, has_id)| n == g && *has_id) {
                        keys.push(icol(g));
                    }
                }
                let group =
                    if keys.is_empty() { String::new() } else { format!(" GROUP BY {}", keys.join(", ")) };
                depth += 1;
                p.sql = format!("SELECT {} FROM ({}) s{depth}{}", sel.join(", "), p.sql, group);
                p.cols = cols;
                p.attrs.clear();
            }
            Stage::Fetch(doc) => {
                depth += 1;
                let outer = format!("s{depth}");
                let mut c = Compiler::new(gid);
                for (v, has_id) in &p.cols {
                    c.seed_column(
                        v,
                        format!("{outer}.{}", vcol(v)),
                        if *has_id { Some(format!("{outer}.{}", icol(v))) } else { None },
                    );
                    if let Some(t) = p.types.get(v) {
                        c.seed_type(v, *t);
                    }
                }
                let obj = fetch_object(gid, &mut c, doc)?;
                p.sql = format!("SELECT {obj} AS row FROM ({}) {outer}", p.sql);
                return Ok(p.sql);
            }
            Stage::Insert(_) | Stage::Put(_) | Stage::Delete(_) | Stage::Update(_) => {
                return Err("write stages do not compile to a read query".into())
            }
        }
    }

    // No fetch: return each row as an object of its variables.
    depth += 1;
    let outer = format!("s{depth}");
    let mut pairs = Vec::new();
    for (v, _) in &p.cols {
        pairs.push(format!("{}, to_jsonb({outer}.{})", compile::lit_str(v), vcol(v)));
    }
    Ok(format!(
        "SELECT jsonb_build_object({}) AS row FROM ({}) {outer}",
        pairs.join(", "),
        p.sql
    ))
}

fn agg_sql(r: &Reduction) -> QResult<String> {
    let arg = r.arg.as_ref().map(|a| vcol(a));
    Ok(match (r.agg.as_str(), arg) {
        ("count", None) => "count(*)".into(),
        ("count", Some(a)) => format!("count({a})"),
        ("sum", Some(a)) => format!("sum(({a})::numeric)"),
        ("max", Some(a)) => format!("max({a})"),
        ("min", Some(a)) => format!("min({a})"),
        ("mean", Some(a)) => format!("avg(({a})::numeric)"),
        ("median", Some(a)) => {
            format!("percentile_cont(0.5) WITHIN GROUP (ORDER BY ({a})::numeric)")
        }
        ("std", Some(a)) => format!("stddev(({a})::numeric)"),
        ("list", Some(a)) => format!("jsonb_agg({a})"),
        (other, None) => return Err(format!("aggregate '{other}' needs an argument")),
        (other, _) => {
            return Err(format!(
                "unknown aggregate '{other}'. supported: count, sum, max, min, mean, \
                 median, std, list"
            ))
        }
    })
}

fn fetch_object(gid: i32, c: &mut Compiler, doc: &FetchDoc) -> QResult<String> {
    let mut pairs = Vec::new();
    for (k, v) in &doc.entries {
        let val = match v {
            FetchVal::Expr(e) => match e {
                Expr::Attr(var, attr) => format!("to_jsonb({})", c.attr_scalar(var, attr)?),
                other => format!("to_jsonb({})", c.expr(other)?),
            },
            FetchVal::AttrList(var, attr) => c.attr_list(var, attr)?,
            FetchVal::Doc(d) => fetch_object(gid, c, d)?,
            FetchVal::SubFetch(sub) => sub_fetch(gid, c, sub)?,
        };
        pairs.push(format!("{}, {val}", compile::lit_str(k)));
    }
    Ok(format!("jsonb_build_object({})", pairs.join(", ")))
}

/// A sub-fetch is a correlated aggregate, which is exactly why it cannot remove
/// an outer row: no match simply yields an empty array (FR-035).
fn sub_fetch(gid: i32, outer: &mut Compiler, q: &Query) -> QResult<String> {
    let mut c = Compiler::nested(outer);
    let mut any = false;
    for s in &q.stages {
        if let Stage::Match(pats) = s {
            c.compile_patterns(pats)?;
            any = true;
        }
    }
    if !any {
        return Err("a sub-fetch needs a 'match' stage".into());
    }
    let Some(Stage::Fetch(doc)) = q.stages.iter().find(|s| matches!(s, Stage::Fetch(_))) else {
        return Err("a sub-fetch needs a 'fetch' stage".into());
    };
    let obj = fetch_object(gid, &mut c, doc)?;
    let from = c.from_sql();
    let where_ = c.where_sql();
    Ok(format!("COALESCE((SELECT jsonb_agg({obj}){from}{where_}), '[]'::jsonb)"))
}

fn exec(sql: &str) -> Vec<Value> {
    Spi::connect(|client| {
        let table = client
            .select(sql, None, &[])
            .unwrap_or_else(|e| error!("typeql execution failed: {e}\n--- compiled SQL ---\n{sql}"));
        table.filter_map(|r| r.get::<JsonB>(1).ok().flatten().map(|j| j.0)).collect()
    })
}

// --------------------------------------------------------------------------
// Write pipeline
// --------------------------------------------------------------------------

fn run_write_pipeline(gid: i32, q: &Query) -> QResult<Vec<Value>> {
    // Bindings come from the leading match, if any.
    let has_match = q.stages.iter().any(|s| matches!(s, Stage::Match(_)));
    let mut envs: Vec<Env> = if has_match {
        let read = Query {
            stages: q.stages.iter().filter(|s| matches!(s, Stage::Match(_))).cloned().collect(),
        };
        let p = compile_match(gid, &read)?;
        let sql = format!("SELECT to_jsonb(t) AS row FROM ({}) t", p.sql);
        let rows = exec(&sql);
        let attrs = p.attrs.clone();
        rows.into_iter()
            .map(|r| {
                let mut env = Env::default();
                for (v, has_id) in &p.cols {
                    let val = r.get(format!("v_{v}")).cloned().unwrap_or(Value::Null);
                    let id = r.get(format!("i_{v}")).and_then(|x| x.as_i64());
                    let ev = match (id, has_id, attrs.contains(v)) {
                        (Some(i), true, true) => EnvVal::Attr(i, val),
                        (Some(i), true, false) => EnvVal::Instance(i),
                        _ => EnvVal::Value(val),
                    };
                    env.vars.insert(v.clone(), ev);
                }
                env
            })
            .collect()
    } else {
        vec![Env::default()]
    };

    let mut written = 0i64;
    for env in envs.iter_mut() {
        for stage in &q.stages {
            match stage {
                Stage::Match(_) | Stage::Define(_) => {}
                Stage::Insert(pats) => {
                    write::run_insert(gid, pats, env)?;
                    written += 1;
                }
                Stage::Put(pats) => {
                    write::run_put(gid, pats, env)?;
                    written += 1;
                }
                Stage::Update(pats) => {
                    write::run_update(gid, pats, env)?;
                    written += 1;
                }
                Stage::Delete(items) => {
                    write::run_delete(gid, items, env)?;
                    written += 1;
                }
                other => {
                    return Err(format!(
                        "'{}' after a write stage is not supported yet: run it as a \
                         separate query",
                        stage_name(other)
                    ))
                }
            }
        }
    }
    Ok(vec![json!({ "rows": envs.len(), "operations": written })])
}

fn stage_name(s: &Stage) -> &'static str {
    match s {
        Stage::Define(_) => "define",
        Stage::Undefine(_) => "undefine",
        Stage::Match(_) => "match",
        Stage::Insert(_) => "insert",
        Stage::Put(_) => "put",
        Stage::Update(_) => "update",
        Stage::Delete(_) => "delete",
        Stage::Select(_) => "select",
        Stage::Sort(_) => "sort",
        Stage::Limit(_) => "limit",
        Stage::Offset(_) => "offset",
        Stage::Distinct => "distinct",
        Stage::Reduce { .. } => "reduce",
        Stage::Fetch(_) => "fetch",
    }
}
