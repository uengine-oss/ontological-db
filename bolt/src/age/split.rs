//! What AGE's Cypher has no clause for, run as several statements in one
//! transaction.
//!
//! AGE's grammar has no `FOREACH` (apache/age #2381), no `CALL { … }` (#1690),
//! and a vertex there has exactly one label, fixed by the table it lives in, so
//! `SET n:L` / `REMOVE n:L` have nothing to change. Robo Architect sends all
//! three (`FEATURE-COMPARISON.md` §4, R3). A statement holding one is cut at
//! it:
//!
//! 1. the clauses before it run as one `cypher()` call that returns every
//!    variable in scope;
//! 2. the construct runs on those rows —
//!    - `FOREACH (x IN list | writes)`: the writes, for every row and element,
//!      as `UNWIND $rows … UNWIND list AS x writes`. The usual
//!      `CASE WHEN c THEN [v] ELSE [] END` list becomes `WITH … WHERE c` with
//!      `x` read as `v`. The rows themselves go on unchanged, as they do in
//!      Neo4j;
//!    - `CALL { … }`: the body once per row, with the imported variables bound
//!      (an aggregate over no rows still gives its one row, as in Neo4j); the
//!      row goes on once per row the body returns, or as it was for a unit
//!      subquery;
//!    - `SET n:L` / `REMOVE n:L`: the vertex is moved to the label's table —
//!      its row deleted from the old one, inserted into the new one with its
//!      properties, and every edge's `start_id`/`end_id` pointed at the new id.
//!      Its id changes (the label is part of AGE's graphid). A vertex can end
//!      with one label only; asking for two is refused;
//! 3. the clauses after it run on the rows that came out, each row's vertices
//!    and edges found again by id.
//!
//! The rows travel as one parameter, `$__og_rows`: per row, a vertex or edge
//! as its id and anything else as its value. A value that holds vertices or
//! edges inside a list or map cannot be found again by id, and is refused.

use super::agtype::Ag;
use super::cypher::{scope_after, Printer, FRESH};
use super::{sql_ident, sql_lit, Refusal, Stats};
use cypher_syntax::ast::*;
use serde_json::{Map, Value as Json};

/// What the steps need of the session: run a `cypher()` call or plain SQL on
/// its connection, inside its transaction.
pub trait Exec {
    /// One AGE Cypher text, `$1` = `params`; the rows' `ncols` values.
    fn cypher(&mut self, text: &str, ncols: usize, params: &Json) -> Result<Vec<Vec<Ag>>, Refusal>;
    /// Plain SQL; every column read as text (NULL as None).
    fn sql(&mut self, sql: &str) -> Result<Vec<Vec<Option<String>>>, Refusal>;
    fn graph(&self) -> String;
}

const ROWS: &str = "__og_rows";
const ROW: &str = "__og_row";

enum Seg {
    /// Ordinary clauses, and the variables in scope after them.
    Clauses(Vec<Clause>, Vec<String>),
    Foreach { var: String, list: Expr, body: Vec<Clause> },
    Call { body: Query, imports: Option<Vec<String>>, optional: bool, returns: Vec<String> },
    /// Per variable: labels added, labels removed.
    Relabel(Vec<(String, Vec<String>, Vec<String>)>),
}

pub struct Steps {
    /// The statement's result columns.
    pub ncols: usize,
    segs: Vec<Seg>,
    now: String,
}

fn has_label_op(items: &[SetOp]) -> bool {
    items.iter().any(|i| matches!(i, SetOp::Label { .. }))
}

/// Does this statement need to be cut?
pub fn needed(q: &Query) -> bool {
    q.clauses.iter().any(|c| match c {
        Clause::Foreach { .. } | Clause::Subquery { .. } => true,
        Clause::Set(items) | Clause::Remove(items) => has_label_op(items),
        Clause::Merge { on_create, on_match, .. } => has_label_op(on_create) || has_label_op(on_match),
        _ => false,
    }) || q.clauses.windows(2).any(|w| merge_then_set(&w[0], &w[1]))
}

/// A MERGE with ON CREATE / ON MATCH items, then a SET or REMOVE: AGE 1.8.0
/// does not store that SET when the branch that ran had items of its own
/// (the statement returns the new values; the table keeps the old ones).
/// Moving the SET into both branches is not enough either — reading the
/// merged relationship's own properties there (`ON MATCH SET r.x =
/// coalesce(r.x, …)`) fails with "binary container must be a VLE vpc" — so the
/// statement is cut after the MERGE and the SET runs as the next statement,
/// on the same rows.
fn merge_then_set(a: &Clause, b: &Clause) -> bool {
    matches!(a, Clause::Merge { on_create, on_match, .. } if !on_create.is_empty() || !on_match.is_empty())
        && matches!(b, Clause::Set(_) | Clause::Remove(_))
}

/// Cut the statement into steps.
pub fn plan(q: &Query, p: &mut Printer) -> Result<Steps, String> {
    if q.union.is_some() {
        return Err(super::cypher::unsupported("UNION with FOREACH, CALL { … } or a label change"));
    }
    let ncols = super::cypher::columns(q).len();
    let mut segs: Vec<Seg> = Vec::new();
    let mut cur: Vec<Clause> = Vec::new();
    let mut scope: Vec<String> = Vec::new();
    let flush = |cur: &mut Vec<Clause>, segs: &mut Vec<Seg>, scope: &[String]| {
        if !cur.is_empty() {
            segs.push(Seg::Clauses(std::mem::take(cur), scope.to_vec()));
        }
    };
    let mut i = 0;
    while i < q.clauses.len() {
        let c = &q.clauses[i];
        match c {
            Clause::Foreach { var, list, body } => {
                if body.iter().any(|b| matches!(b, Clause::Foreach { .. }) || matches!(b, Clause::Set(x) | Clause::Remove(x) if has_label_op(x))) {
                    return Err(super::cypher::unsupported("FOREACH inside FOREACH, or a label change inside FOREACH"));
                }
                flush(&mut cur, &mut segs, &scope);
                segs.push(Seg::Foreach { var: var.clone(), list: list.clone(), body: body.clone() });
            }
            Clause::Subquery { scope: sc, body, optional } => {
                if needed(body) {
                    return Err(super::cypher::unsupported("FOREACH, CALL { … } or a label change inside CALL { … }"));
                }
                flush(&mut cur, &mut segs, &scope);
                let imports = match sc {
                    Some(names) if names.iter().any(|n| n == "*") => None,
                    Some(names) => Some(names.clone()),
                    // Neo4j 4: a leading `WITH a, b` imports.
                    None => Some(match body.clauses.first() {
                        Some(Clause::With { proj, .. }) => proj
                            .items
                            .iter()
                            .filter_map(|it| match &it.expr {
                                Expr::Var(v) => Some(v.clone()),
                                _ => None,
                            })
                            .collect(),
                        _ => Vec::new(),
                    }),
                };
                let returns = super::cypher::columns(body);
                segs.push(Seg::Call { body: (**body).clone(), imports, optional: *optional, returns });
            }
            Clause::Set(items) | Clause::Remove(items) if has_label_op(items) => {
                // This clause and the SET / REMOVE right after it: the label
                // changes are made once, together; the property changes stay
                // in order, before them.
                let mut ops: Vec<(String, Vec<String>, Vec<String>)> = Vec::new();
                let mut j = i;
                while j < q.clauses.len() {
                    let (items, add) = match &q.clauses[j] {
                        Clause::Set(items) => (items, true),
                        Clause::Remove(items) => (items, false),
                        _ => break,
                    };
                    let mut rest = Vec::new();
                    for it in items {
                        match it {
                            SetOp::Label { var, labels } => {
                                let at = match ops.iter().position(|(v, _, _)| v == var) {
                                    Some(k) => k,
                                    None => {
                                        ops.push((var.clone(), Vec::new(), Vec::new()));
                                        ops.len() - 1
                                    }
                                };
                                for l in labels {
                                    let (_, a, r) = &mut ops[at];
                                    if add {
                                        r.retain(|x| x != l);
                                        if !a.contains(l) {
                                            a.push(l.clone());
                                        }
                                    } else {
                                        a.retain(|x| x != l);
                                        if !r.contains(l) {
                                            r.push(l.clone());
                                        }
                                    }
                                }
                            }
                            other => rest.push(other.clone()),
                        }
                    }
                    if !rest.is_empty() {
                        if cur.last().is_some_and(|l| merge_then_set(l, &q.clauses[j])) {
                            flush(&mut cur, &mut segs, &scope);
                        }
                        cur.push(if add { Clause::Set(rest) } else { Clause::Remove(rest) });
                    }
                    j += 1;
                }
                flush(&mut cur, &mut segs, &scope);
                segs.push(Seg::Relabel(ops));
                i = j;
                continue;
            }
            Clause::Merge { on_create, on_match, .. } if has_label_op(on_create) || has_label_op(on_match) => {
                return Err(super::cypher::unsupported("a label set in ON CREATE / ON MATCH"));
            }
            other => {
                if cur.last().is_some_and(|l| merge_then_set(l, other)) {
                    flush(&mut cur, &mut segs, &scope);
                }
                cur.push(other.clone())
            }
        }
        scope = scope_after(&scope, c);
        i += 1;
    }
    flush(&mut cur, &mut segs, &scope);
    Ok(Steps { ncols, segs, now: p.now.clone() })
}

// ------------------------------------------------------------------ rows

/// One row between steps: each variable and its value.
#[derive(Clone, Default, Debug)]
struct Row(Vec<(String, Ag)>);

impl Row {
    fn get(&self, v: &str) -> Option<&Ag> {
        self.0.iter().find(|(k, _)| k == v).map(|(_, a)| a)
    }

    fn set(&mut self, v: &str, a: Ag) {
        match self.0.iter_mut().find(|(k, _)| k == v) {
            Some(slot) => slot.1 = a,
            None => self.0.push((v.to_string(), a)),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Vertex,
    Edge,
    Value,
}

fn holds_entity(a: &Ag) -> bool {
    match a {
        Ag::Vertex { .. } | Ag::Edge { .. } | Ag::Path(_) => true,
        Ag::List(xs) => xs.iter().any(holds_entity),
        Ag::Map(kv) => kv.iter().any(|(_, v)| holds_entity(v)),
        _ => false,
    }
}

/// How each variable travels: by id (vertices, edges) or by value. A
/// vertex's label, when every row's is the same, narrows the lookup.
fn kinds(rows: &[Row], vars: &[String]) -> Result<Vec<(String, Kind, Option<String>)>, Refusal> {
    let mut out = Vec::new();
    for v in vars {
        let mut kind = Kind::Value;
        let mut label: Option<Option<String>> = None;
        for r in rows {
            match r.get(v) {
                Some(Ag::Vertex { label: l, .. }) => {
                    kind = Kind::Vertex;
                    label = match label {
                        None => Some(Some(l.clone())),
                        Some(Some(x)) if x == *l => Some(Some(x)),
                        _ => Some(None),
                    };
                }
                Some(Ag::Edge { .. }) => kind = Kind::Edge,
                Some(a) if holds_entity(a) => {
                    return Err(Refusal::syntax(super::cypher::unsupported(&format!(
                        "carrying `{v}` (a path, or a list or map of nodes) across FOREACH, CALL {{ … }} or a label change"
                    ))))
                }
                _ => {}
            }
        }
        let label = label.flatten().filter(|l| !l.is_empty() && kind == Kind::Vertex);
        out.push((v.clone(), kind, label));
    }
    Ok(out)
}

/// The rows as the `$__og_rows` parameter.
fn rows_param(rows: &[Row], kinds: &[(String, Kind, Option<String>)]) -> Json {
    Json::Array(
        rows.iter()
            .enumerate()
            .map(|(i, r)| {
                let mut m = Map::new();
                m.insert("__i".into(), Json::from(i as i64));
                for (v, _, _) in kinds {
                    let val = match r.get(v) {
                        Some(a) => match a.element_id() {
                            Some(id) => Json::from(id),
                            None => a.to_json(),
                        },
                        None => Json::Null,
                    };
                    m.insert(v.clone(), val);
                }
                Json::Object(m)
            })
            .collect(),
    )
}

/// `UNWIND $__og_rows AS __og_row` and each variable bound again.
fn prelude(kinds: &[(String, Kind, Option<String>)]) -> String {
    use super::cypher::ident;
    let mut s = format!("UNWIND ${ROWS} AS {ROW}");
    let mut carry = vec![ROW.to_string()];
    for (v, k, label) in kinds {
        let key = format!("{ROW}.{}", ident(v));
        match k {
            Kind::Vertex => {
                let l = label.as_ref().map(|l| format!(":{}", ident(l))).unwrap_or_default();
                s.push_str(&format!(" OPTIONAL MATCH ({}{l}) WHERE id({}) = {key}", ident(v), ident(v)));
                carry.push(ident(v));
            }
            Kind::Edge => {
                s.push_str(&format!(" OPTIONAL MATCH ()-[{}]->() WHERE id({}) = {key}", ident(v), ident(v)));
                carry.push(ident(v));
            }
            Kind::Value => carry.push(format!("{key} AS {}", ident(v))),
        }
    }
    s.push_str(&format!(" WITH {}", carry.join(", ")));
    s
}

fn with_param(params: &Json, name: &str, v: Json) -> Json {
    let mut m = params.as_object().cloned().unwrap_or_default();
    m.insert(name.into(), v);
    Json::Object(m)
}

fn merged(params: &Json, extra: &Map<String, Json>) -> Json {
    let mut m = params.as_object().cloned().unwrap_or_default();
    for (k, v) in extra {
        m.insert(k.clone(), v.clone());
    }
    Json::Object(m)
}

/// `CASE WHEN c THEN [e] ELSE [] END` (or the other way round): the condition
/// under which FOREACH runs once, and the one element.
fn conditional(list: &Expr) -> Option<(Expr, Expr)> {
    let Expr::Case { operand: None, whens, else_ } = list else { return None };
    let [(cond, then)] = whens.as_slice() else { return None };
    let empty = |e: &Expr| matches!(e, Expr::List(xs) if xs.is_empty());
    let one = |e: &Expr| match e {
        Expr::List(xs) if xs.len() == 1 => Some(xs[0].clone()),
        _ => None,
    };
    let else_ = else_.as_deref().cloned().unwrap_or(Expr::Lit(Lit::Null));
    if let (Some(e), true) = (one(then), empty(&else_)) {
        return Some((cond.clone(), e));
    }
    if let (true, Some(e)) = (empty(then), one(&else_)) {
        return Some((Expr::Not(Box::new(cond.clone())), e));
    }
    None
}

/// Every `Var(from)` in the clauses read as `Var(to)`.
fn rename_clauses(cs: &mut [Clause], from: &str, to: &str) {
    fn e(x: &mut Expr, f: &str, t: &str) {
        match x {
            Expr::Var(v) if v == f => *v = t.to_string(),
            Expr::Prop(b, _) | Expr::Not(b) | Expr::Neg(b) | Expr::IsNull(b, _) => e(b, f, t),
            Expr::Binary(_, a, b) => {
                e(a, f, t);
                e(b, f, t)
            }
            Expr::Func { args, .. } | Expr::List(args) => args.iter_mut().for_each(|a| e(a, f, t)),
            Expr::Map(kv) => kv.iter_mut().for_each(|(_, v)| e(v, f, t)),
            Expr::Case { operand, whens, else_ } => {
                if let Some(o) = operand {
                    e(o, f, t);
                }
                for (a, b) in whens {
                    e(a, f, t);
                    e(b, f, t);
                }
                if let Some(x) = else_ {
                    e(x, f, t);
                }
            }
            Expr::MapProjection { var, items } => {
                if var == f {
                    *var = t.to_string();
                }
                for it in items {
                    if let MapProjItem::Entry(_, v) = it {
                        e(v, f, t);
                    }
                }
            }
            Expr::HasLabels { var, .. } if var == f => *var = t.to_string(),
            Expr::Slice { list, from, to } => {
                e(list, f, t);
                if let Some(x) = from {
                    e(x, f, t);
                }
                if let Some(x) = to {
                    e(x, f, t);
                }
            }
            Expr::ListComp { source, filter, project, .. } => {
                e(source, f, t);
                if let Some(x) = filter {
                    e(x, f, t);
                }
                if let Some(x) = project {
                    e(x, f, t);
                }
            }
            Expr::ListPred { source, filter, .. } => {
                e(source, f, t);
                e(filter, f, t);
            }
            Expr::Reduce { init, list, expr, .. } => {
                e(init, f, t);
                e(list, f, t);
                e(expr, f, t);
            }
            _ => {}
        }
    }
    fn pat(p: &mut Pattern, f: &str, t: &str) {
        for el in &mut p.elems {
            match el {
                PatElem::Node(n) => {
                    if n.var.as_deref() == Some(f) {
                        n.var = Some(t.to_string());
                    }
                    n.props.iter_mut().for_each(|(_, v)| e(v, f, t));
                }
                PatElem::Rel(r) => {
                    if r.var.as_deref() == Some(f) {
                        r.var = Some(t.to_string());
                    }
                    r.props.iter_mut().for_each(|(_, v)| e(v, f, t));
                }
            }
        }
    }
    fn ops(items: &mut [SetOp], f: &str, t: &str) {
        for it in items {
            match it {
                SetOp::Prop { var, value, .. } => {
                    if var == f {
                        *var = t.to_string();
                    }
                    e(value, f, t);
                }
                SetOp::Map { var, value, .. } => {
                    if var == f {
                        *var = t.to_string();
                    }
                    e(value, f, t);
                }
                SetOp::Label { var, .. } => {
                    if var == f {
                        *var = t.to_string();
                    }
                }
            }
        }
    }
    for c in cs {
        match c {
            Clause::Create(ps) => ps.iter_mut().for_each(|p| pat(p, from, to)),
            Clause::Merge { pattern, on_create, on_match } => {
                pat(pattern, from, to);
                ops(on_create, from, to);
                ops(on_match, from, to);
            }
            Clause::Set(items) | Clause::Remove(items) => ops(items, from, to),
            Clause::Delete { exprs, .. } => exprs.iter_mut().for_each(|x| e(x, from, to)),
            _ => {}
        }
    }
}

fn row_of(names: &[String], vals: Vec<Ag>) -> Row {
    Row(names.iter().cloned().zip(vals).collect())
}

/// Run the steps. Returns the statement's rows (its RETURN columns) and the
/// counter corrections for vertices that changed table.
pub fn run(x: &mut dyn Exec, steps: &Steps, params: &Json) -> Result<(Vec<Vec<Ag>>, Stats), Refusal> {
    let pmap = params.as_object().cloned().unwrap_or_default();
    let mut rows: Vec<Row> = vec![Row::default()];
    let mut scope: Vec<String> = Vec::new();
    let mut adjust = Stats::default();
    let mut out: Vec<Vec<Ag>> = Vec::new();
    let mut first = true;
    let last = steps.segs.len().saturating_sub(1);
    for (k, seg) in steps.segs.iter().enumerate() {
        let mut printer = Printer::new(Some(&pmap), steps.now.clone());
        let ks = kinds(&rows, &scope)?;
        let pre = if first { None } else { Some(prelude(&ks)) };
        let rows_json = rows_param(&rows, &ks);
        match seg {
            Seg::Clauses(cs, after) => {
                let mut texts = Vec::new();
                if let Some(p) = &pre {
                    texts.push(p.clone());
                }
                let mut sc = scope.clone();
                for c in cs {
                    texts.push(printer.clause(c, &sc).map_err(Refusal::syntax)?);
                    sc = scope_after(&sc, c);
                }
                if k == last {
                    let text = texts.join(" ");
                    let p = with_param(&merged(params, &printer.extra), ROWS, rows_json);
                    out = x.cypher(&text, steps.ncols, &p)?;
                    return Ok((out, adjust));
                }
                // carry every variable in scope to the next step
                let carry: Vec<String> = after.iter().filter(|v| !v.starts_with(FRESH) || true).cloned().collect();
                let text = if carry.is_empty() {
                    format!("{} RETURN 1 AS {FRESH}one", texts.join(" "))
                } else {
                    format!("{} RETURN {}", texts.join(" "), carry.iter().map(|v| super::cypher::ident(v)).collect::<Vec<_>>().join(", "))
                };
                let p = with_param(&merged(params, &printer.extra), ROWS, rows_json);
                let got = x.cypher(&text, carry.len().max(1), &p)?;
                rows = got.into_iter().map(|vals| if carry.is_empty() { Row::default() } else { row_of(&carry, vals) }).collect();
                scope = after.clone();
            }
            Seg::Foreach { var, list, body } => {
                let mut text = pre.clone().unwrap_or_else(|| format!("UNWIND ${ROWS} AS {ROW}"));
                let mut body = body.clone();
                match conditional(list) {
                    Some((cond, elem)) => {
                        let mut carry: Vec<String> = ks.iter().map(|(v, _, _)| super::cypher::ident(v)).collect();
                        match &elem {
                            Expr::Var(v) => rename_clauses(&mut body, var, v),
                            other => carry.push(format!("{} AS {}", printer.expr(other).map_err(Refusal::syntax)?, super::cypher::ident(var))),
                        }
                        if carry.is_empty() {
                            carry.push(format!("1 AS {FRESH}one"));
                        }
                        text.push_str(&format!(" WITH {} WHERE {}", carry.join(", "), printer.expr(&cond).map_err(Refusal::syntax)?));
                    }
                    None => text.push_str(&format!(" UNWIND {} AS {}", printer.operand(list).map_err(Refusal::syntax)?, super::cypher::ident(var))),
                }
                let mut sc = scope.clone();
                sc.push(var.clone());
                for c in &body {
                    text.push(' ');
                    text.push_str(&printer.clause(c, &sc).map_err(Refusal::syntax)?);
                }
                let p = with_param(&merged(params, &printer.extra), ROWS, rows_json);
                x.cypher(&text, 0, &p)?;
                // The rows go on as they were; their elements are found again
                // by id, so the next step sees what the writes did.
                if first {
                    first = false;
                }
                if k == last {
                    return Ok((Vec::new(), adjust));
                }
                continue;
            }
            Seg::Call { body, imports, optional, returns } => {
                let imported: Vec<(String, Kind, Option<String>)> = match imports {
                    None => ks.clone(),
                    Some(names) => ks.iter().filter(|(v, _, _)| names.contains(v)).cloned().collect(),
                };
                let body_text = printer.query(body).map_err(Refusal::syntax)?;
                let mut next = Vec::new();
                for (i, r) in rows.iter().enumerate() {
                    let one = rows_param(std::slice::from_ref(r), &imported);
                    let mut text = prelude(&imported);
                    // the row's index, as the prelude's UNWIND expects it
                    let one = match one {
                        Json::Array(mut xs) => {
                            if let Some(Json::Object(m)) = xs.get_mut(0) {
                                m.insert("__i".into(), Json::from(i as i64));
                            }
                            Json::Array(xs)
                        }
                        o => o,
                    };
                    text.push(' ');
                    text.push_str(&body_text);
                    let p = with_param(&merged(params, &printer.extra), ROWS, one);
                    let got = x.cypher(&text, returns.len(), &p)?;
                    if returns.is_empty() {
                        next.push(r.clone());
                        continue;
                    }
                    if got.is_empty() && *optional {
                        let mut nr = r.clone();
                        for n in returns {
                            nr.set(n, Ag::Null);
                        }
                        next.push(nr);
                    }
                    for vals in got {
                        let mut nr = r.clone();
                        for (n, v) in returns.iter().zip(vals) {
                            nr.set(n, v);
                        }
                        next.push(nr);
                    }
                }
                rows = next;
                for n in returns {
                    if !scope.contains(n) {
                        scope.push(n.clone());
                    }
                }
            }
            Seg::Relabel(ops) => {
                let graph = x.graph();
                for r in rows.iter_mut() {
                    for (var, add, remove) in ops {
                        let Some(Ag::Vertex { id, label, props }) = r.get(var).cloned() else {
                            if matches!(r.get(var), None | Some(Ag::Null)) {
                                continue;
                            }
                            return Err(Refusal::syntax(format!("SET/REMOVE of a label needs a node; `{var}` is not one")));
                        };
                        let current = if label.is_empty() || label == "_ag_label_vertex" { Vec::new() } else { vec![label.clone()] };
                        let mut want: Vec<String> = current.iter().filter(|l| !remove.contains(l)).cloned().collect();
                        for a in add {
                            if !want.contains(a) {
                                want.push(a.clone());
                            }
                        }
                        let added = add.iter().filter(|a| !current.contains(a)).count() as i64;
                        let removed = remove.iter().filter(|l| current.contains(l)).count() as i64;
                        if want.len() > 1 {
                            return Err(Refusal {
                                code: "Neo.ClientError.Statement.SemanticError".into(),
                                message: format!(
                                    "a vertex in AGE has one label: `{var}` would have {} — set the new one and remove the old one in the same statement",
                                    want.join(", ")
                                ),
                            });
                        }
                        let target = want.first().cloned().unwrap_or_else(|| "_ag_label_vertex".into());
                        if target == label || (current.is_empty() && want.is_empty()) {
                            continue;
                        }
                        let new_id = relabel(x, &graph, id, &target)?;
                        adjust.nodes_created -= 1;
                        adjust.nodes_deleted -= 1;
                        adjust.labels_added += added - 1;
                        adjust.labels_removed += removed;
                        adjust.properties_set -= new_id.1;
                        // every variable that was this vertex is the new one
                        let moved = Ag::Vertex { id: new_id.0, label: target.clone(), props: props.clone() };
                        for slot in r.0.iter_mut() {
                            if slot.1.element_id() == Some(id) && matches!(slot.1, Ag::Vertex { .. }) {
                                slot.1 = moved.clone();
                            }
                        }
                    }
                }
            }
        }
        first = false;
        if k == last {
            // ended on FOREACH / CALL / a label change: no RETURN after it
            if steps.ncols == 0 {
                return Ok((Vec::new(), adjust));
            }
            return Err(Refusal::syntax("the statement ends without a RETURN".into()));
        }
    }
    Ok((out, adjust))
}

/// Move one vertex to `target`'s table; returns its new id and how many edge
/// rows were pointed at it.
fn relabel(x: &mut dyn Exec, graph: &str, id: i64, target: &str) -> Result<(i64, i64), Refusal> {
    let g = sql_ident(graph);
    if target != "_ag_label_vertex" {
        let have = x.sql(&format!(
            "SELECT l.kind::text FROM ag_catalog.ag_label l JOIN ag_catalog.ag_graph g ON g.graphid = l.graph \
             WHERE g.name = {} AND l.name = {}",
            sql_lit(graph),
            sql_lit(target)
        ))?;
        match have.first().and_then(|r| r[0].clone()) {
            None => {
                x.sql(&format!("SELECT ag_catalog.create_vlabel({}, {})", sql_lit(graph), sql_lit(target)))?;
            }
            Some(k) if k == "v" => {}
            Some(_) => {
                return Err(Refusal::syntax(format!("'{target}' is a relationship type in graph '{graph}', not a label")));
            }
        }
    }
    let got = x.sql(&format!(
        "WITH old AS (DELETE FROM {g}.\"_ag_label_vertex\" WHERE id = '{id}'::ag_catalog.graphid RETURNING properties) \
         INSERT INTO {g}.{} (properties) SELECT properties FROM old RETURNING id::text",
        sql_ident(target)
    ))?;
    let new_id: i64 = got
        .first()
        .and_then(|r| r[0].as_deref())
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| Refusal::syntax(format!("vertex {id} was not found to relabel")))?;
    let moved = x.sql(&format!(
        "WITH u AS (UPDATE {g}.\"_ag_label_edge\" SET \
           start_id = CASE WHEN start_id = '{id}'::ag_catalog.graphid THEN '{new_id}'::ag_catalog.graphid ELSE start_id END, \
           end_id = CASE WHEN end_id = '{id}'::ag_catalog.graphid THEN '{new_id}'::ag_catalog.graphid ELSE end_id END \
         WHERE start_id = '{id}'::ag_catalog.graphid OR end_id = '{id}'::ag_catalog.graphid RETURNING 1) SELECT count(*)::text FROM u"
    ))?;
    let n = moved.first().and_then(|r| r[0].as_deref()).and_then(|s| s.parse().ok()).unwrap_or(0);
    Ok((new_id, n))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records what it was asked and answers from a script.
    struct Fake {
        asked: Vec<String>,
        answers: Vec<Vec<Vec<Ag>>>,
    }

    impl Exec for Fake {
        fn cypher(&mut self, text: &str, _ncols: usize, params: &Json) -> Result<Vec<Vec<Ag>>, Refusal> {
            self.asked.push(format!("{text} | rows={}", params.get(ROWS).map(|r| r.to_string()).unwrap_or_default()));
            Ok(if self.answers.is_empty() { Vec::new() } else { self.answers.remove(0) })
        }
        fn sql(&mut self, sql: &str) -> Result<Vec<Vec<Option<String>>>, Refusal> {
            self.asked.push(sql.to_string());
            Ok(if sql.contains("RETURNING id::text") {
                vec![vec![Some("1407374883553281".into())]]
            } else if sql.contains("count(*)::text") {
                vec![vec![Some("2".into())]]
            } else {
                vec![]
            })
        }
        fn graph(&self) -> String {
            "g".into()
        }
    }

    fn v(id: i64, label: &str) -> Ag {
        Ag::Vertex { id, label: label.into(), props: vec![] }
    }

    fn steps(q: &str) -> Steps {
        let ast = cypher_syntax::parse(q).unwrap();
        assert!(needed(&ast));
        let mut p = Printer::new(None, "2026-11-18T00:00:00+00:00".into());
        plan(&ast, &mut p).unwrap()
    }

    #[test]
    fn a_conditional_foreach_is_a_filtered_write() {
        let s = steps(
            "MATCH (m:M {id: 1}) OPTIONAL MATCH (c:C {id: 2}) \
             FOREACH (cc IN CASE WHEN c IS NULL THEN [] ELSE [c] END | MERGE (m)-[:MAPS]->(cc)) RETURN m",
        );
        let mut f = Fake { asked: vec![], answers: vec![vec![vec![v(10, "M"), v(20, "C")]], vec![], vec![vec![v(10, "M")]]] };
        let (rows, _) = run(&mut f, &s, &Json::Object(Map::new())).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(f.asked[0], "MATCH (m:M {id: 1}) OPTIONAL MATCH (c:C {id: 2}) RETURN m, c | rows=[{\"__i\":0}]");
        assert!(f.asked[1].starts_with("UNWIND $__og_rows AS __og_row OPTIONAL MATCH (m:M) WHERE id(m) = __og_row.m OPTIONAL MATCH (c:C) WHERE id(c) = __og_row.c WITH __og_row, m, c WITH m, c WHERE NOT (c IS NULL) MERGE (m)-[:MAPS]->(c)"), "{}", f.asked[1]);
        assert!(f.asked[1].ends_with(r#"rows=[{"__i":0,"c":20,"m":10}]"#), "{}", f.asked[1]);
        assert!(f.asked[2].ends_with("WITH __og_row, m, c RETURN m | rows=[{\"__i\":0,\"c\":20,\"m\":10}]"), "{}", f.asked[2]);
    }

    #[test]
    fn a_call_subquery_runs_per_row() {
        let s = steps(
            "MATCH (e {id: $id}) CALL { WITH e UNWIND $files AS f MERGE (i:F {path: f.path}) MERGE (e)-[:IN]->(i) RETURN count(*) AS merged } \
             OPTIONAL MATCH (e)-[r:IN]->() RETURN count(r) AS n",
        );
        let mut f = Fake {
            asked: vec![],
            answers: vec![vec![vec![v(1, "A")], vec![v(2, "A")]], vec![vec![Ag::Int(2)]], vec![vec![Ag::Int(0)]], vec![vec![Ag::Int(2)]]],
        };
        let (rows, _) = run(&mut f, &s, &serde_json::json!({"id": "x", "files": []})).unwrap();
        assert_eq!(rows, vec![vec![Ag::Int(2)]]);
        assert_eq!(f.asked.len(), 4, "{:?}", f.asked);
        assert!(f.asked[1].contains("OPTIONAL MATCH (e:A) WHERE id(e) = __og_row.e WITH __og_row, e WITH e UNWIND $files AS f"), "{}", f.asked[1]);
        assert!(f.asked[1].ends_with(r#"rows=[{"__i":0,"e":1}]"#), "{}", f.asked[1]);
        assert!(f.asked[3].contains(r#"WITH __og_row, e, __og_row.merged AS merged OPTIONAL MATCH (e)-[r:`IN`]->() RETURN count(r) AS n"#), "{}", f.asked[3]);
        assert!(f.asked[3].contains(r#""merged":2"#) && f.asked[3].contains(r#""merged":0"#), "{}", f.asked[3]);
    }

    #[test]
    fn a_label_change_moves_the_vertex() {
        let s = steps("MATCH (n:Process) WHERE n.x IS NULL SET n:BpmnProcess, n.s = $sid REMOVE n:Process RETURN count(n) AS c");
        let mut f = Fake { asked: vec![], answers: vec![vec![vec![v(844424930131969, "Process")]], vec![vec![Ag::Int(1)]]] };
        let (rows, adj) = run(&mut f, &s, &serde_json::json!({"sid": "s"})).unwrap();
        assert_eq!(rows, vec![vec![Ag::Int(1)]]);
        assert_eq!(f.asked[0], "MATCH (n:Process) WHERE n.x IS NULL SET n.s = $sid RETURN n | rows=[{\"__i\":0}]");
        assert!(f.asked[2].contains("SELECT ag_catalog.create_vlabel('g', 'BpmnProcess')"), "{:?}", f.asked);
        assert!(f.asked[3].contains("DELETE FROM \"g\".\"_ag_label_vertex\" WHERE id = '844424930131969'::ag_catalog.graphid"), "{}", f.asked[3]);
        assert!(f.asked[5].contains("OPTIONAL MATCH (n:BpmnProcess) WHERE id(n) = __og_row.n") && f.asked[5].contains("1407374883553281"), "{}", f.asked[5]);
        assert_eq!((adj.nodes_created, adj.nodes_deleted, adj.labels_added, adj.labels_removed, adj.properties_set), (-1, -1, 0, 1, -2));
    }

    #[test]
    fn a_set_after_merge_runs_as_its_own_statement() {
        let s = steps("UNWIND $rows AS r MATCH (a:A {id: r.a}) MERGE (a)-[e:R]->(b:B {k: 1}) ON CREATE SET e.x = r.x SET e.x = coalesce(e.x, 0) RETURN e.x AS x");
        let e = Ag::Edge { id: 7, label: "R".into(), start: 1, end: 2, props: vec![] };
        let mut f = Fake { asked: vec![], answers: vec![vec![vec![Ag::Map(vec![("a".into(), Ag::Int(1))]), v(1, "A"), e, v(2, "B")]], vec![vec![Ag::Int(5)]]] };
        let (rows, _) = run(&mut f, &s, &serde_json::json!({"rows": [{"a": 1}]})).unwrap();
        assert_eq!(rows, vec![vec![Ag::Int(5)]]);
        assert!(f.asked[0].starts_with("UNWIND $rows AS r MATCH (a:A {id: r.a}) MERGE (a)-[e:R]->(b:B {k: 1}) ON CREATE SET e.x = r.x RETURN r, a, e, b |"), "{}", f.asked[0]);
        assert!(f.asked[1].contains("OPTIONAL MATCH ()-[e]->() WHERE id(e) = __og_row.e") && f.asked[1].contains("SET e.x = coalesce(e.x, 0) RETURN e.x AS x"), "{}", f.asked[1]);
    }

    #[test]
    fn two_labels_are_refused() {
        let s = steps("MATCH (n:A) SET n:B RETURN n");
        let mut f = Fake { asked: vec![], answers: vec![vec![vec![v(1, "A")]]] };
        let e = run(&mut f, &s, &Json::Null).err().unwrap();
        assert!(e.message.contains("one label"), "{}", e.message);
    }
}
