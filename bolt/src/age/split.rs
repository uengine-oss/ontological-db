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
//!      Its id changes (the label is part of AGE's graphid). A vertex has the
//!      labels of its table and of the tables that one inherits
//!      (`hierarchy.rs`): setting one of those is nothing to do, removing one
//!      is refused, and a new table takes the old one's parents
//!      (`REMOVE n:Customer SET n:Client` makes `Client` inherit `_Entity`).
//!      Otherwise a vertex ends with one label only; asking for two is
//!      refused;
//! 3. the clauses after it run on the rows that came out, each row's vertices
//!    and edges found again by id.
//!
//! The rows travel as one parameter, `$__og_rows`: per row, a vertex or edge
//! as its id and anything else as its value. A value that holds vertices or
//! edges inside a list or map cannot be found again by id, and is refused.
//!
//! The same cutting avoids answers AGE 1.8.0 gets wrong within one statement
//! (SUITES-ON-AGE §5; each shape is in [`Cut`], [`merge_moves`] and
//! [`waves`]):
//!
//! - **N4** a WHERE after a SET that AGE would evaluate below the SET — cut
//!   before the WHERE's clause;
//! - **N3** a MATCH after a CREATE / MERGE that could see what was written —
//!   cut before the MATCH;
//! - **N1** a MERGE whose ON CREATE / ON MATCH items read what it merges —
//!   the MERGE marks what it creates, the items run after it, as two steps on
//!   the rows that matched and on those that created;
//! - **N2** a step that writes an element more than one row reaches — run in
//!   waves, one row per element at a time, in row order.

use super::agtype::Ag;
use super::cypher::{scope_after, Printer, FRESH};
use super::hierarchy::{self, Hierarchy, Move, Schema};
use super::{sql_ident, sql_lit, Refusal, Stats};
use cypher_syntax::ast::*;
use serde_json::{Map, Value as Json};
use std::collections::{HashMap, HashSet};

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
    /// N1: a MERGE's ON CREATE / ON MATCH items, taken out of it because they
    /// read what it merged. The MERGE before this marked what it made
    /// (`ON CREATE SET <marker>.__og_c = true`); here the items run on the
    /// rows that matched and on those that created, the marker goes again.
    MergeSets { marker: String, on_create: Vec<SetOp>, on_match: Vec<SetOp> },
}

pub struct Steps {
    /// The statement's result columns.
    pub ncols: usize,
    segs: Vec<Seg>,
    now: String,
    /// The label hierarchy the steps are printed with; a label change that
    /// adds inheritance adds it here too, for the steps after it.
    hier: Hierarchy,
}

fn has_label_op(items: &[SetOp]) -> bool {
    items.iter().any(|i| matches!(i, SetOp::Label { .. }))
}

/// Does this statement need to be cut — for a construct AGE has no clause
/// for, or a MERGE followed by a SET? (What the gateway has always cut; the
/// cuts for AGE's wrong answers, [`Cut`] and [`merge_moves`], come on top in
/// [`needed`]. CALL { … } bodies and the walk in `reach.rs` go by this one.)
pub fn must_cut(q: &Query) -> bool {
    q.clauses.iter().any(|c| match c {
        Clause::Foreach { .. } | Clause::Subquery { .. } => true,
        Clause::Set(items) | Clause::Remove(items) => has_label_op(items),
        Clause::Merge { on_create, on_match, .. } => has_label_op(on_create) || has_label_op(on_match),
        _ => false,
    }) || q.clauses.windows(2).any(|w| merge_then_set(&w[0], &w[1]))
}

/// Does this statement need to be cut? [`must_cut`], or one of the shapes AGE
/// answers wrongly in one statement (N1, N3, N4). `h`: what a write's labels
/// bring with them (N3).
pub fn needed(q: &Query, h: &Hierarchy) -> bool {
    if must_cut(q) {
        return true;
    }
    if q.union.is_some() {
        return false;
    }
    let mut cut = Cut::default();
    let mut scope: Vec<String> = Vec::new();
    for (i, c) in q.clauses.iter().enumerate() {
        if (i > 0 && cut.before(c, &scope)) || merge_moves(c, &scope).is_some() {
            return true;
        }
        cut.after(c, &scope, h);
        scope = scope_after(&scope, c);
    }
    false
}

/// Where AGE 1.8.0 answers wrongly unless the statement is cut, decided
/// clause by clause over one part of a statement (reset at every cut).
#[derive(Default)]
struct Cut {
    /// N4: a property SET or REMOVE so far.
    set: bool,
    /// N4: the variables a filter may be put below the SET by: those in scope
    /// at a SET, and what later projections make of them alone (or of
    /// nothing) — not what UNWIND or MATCH bring after it.
    below: Vec<String>,
    /// N3: the labels of the vertices CREATE / MERGE wrote so far (`""` for
    /// none) — each with the labels its table inherits, which a MATCH of those
    /// scans too — and the types of the relationships.
    labels: Vec<String>,
    types: Vec<String>,
}

impl Cut {
    /// Cut before `c` (scope: the variables before it)?
    ///
    /// - **N4**: after a SET / REMOVE, a WHERE (of a WITH, MATCH or OPTIONAL
    ///   MATCH) with a part that reads only what was there at the SET
    ///   ([`Cut::below`]), or nothing. AGE pushes such a filter below the SET:
    ///   `MATCH (s:V) SET s.v = s.k * 10 WITH s WHERE s.v > 10 RETURN
    ///   collect(s.v)` filters on the old `v` (`[]`; Neo4j `[20, 30]`), and the
    ///   rows it filtered out are not written at all — the plan is `Cypher Set
    ///   -> Subquery Scan, Filter: v > 10`. Cut, the filter runs in the next
    ///   statement on the rows carried, after every row's SET. A WHERE on what
    ///   came after the SET (Robo's `OPTIONAL MATCH (a:Aggregate {id: $id})
    ///   WHERE a IS NOT NULL`, `UNWIND refs AS ref WITH g, ref WHERE ref.id IS
    ///   NOT NULL`) cannot go below it and is left.
    /// - **N3**: after a CREATE / MERGE, a MATCH or OPTIONAL MATCH whose new
    ///   elements can see what was written: a label or type written before it
    ///   in this part, or a node without a label / a relationship without a
    ///   type. AGE's scan sees part of what the same statement wrote:
    ///   `UNWIND [1, 2, 3] AS i CREATE (s:V {k: i}) WITH count(s) AS made
    ///   MATCH (t:V) RETURN made, count(t)` gives `3 | 1` (Neo4j `3 | 3`).
    ///   A MATCH of labels and types not written here stays in the statement
    ///   (Robo's `MERGE (a:X …) WITH a MATCH (b:Y {id: …})`).
    fn before(&self, c: &Clause, scope: &[String]) -> bool {
        let seen = || scope_after(scope, c);
        match c {
            Clause::With { proj, where_: Some(w) } => self.set && filters_old_rows(w, &self.project(proj), &seen()),
            Clause::Match { patterns, where_, .. } => {
                (self.set && where_.as_ref().is_some_and(|w| filters_old_rows(w, &self.below, &seen())))
                    || self.sees_writes(patterns, scope)
            }
            _ => false,
        }
    }

    /// [`Cut::below`] after a projection: the items made of those alone.
    fn project(&self, proj: &Projection) -> Vec<String> {
        let mut out = Vec::new();
        for it in &proj.items {
            if matches!(it.expr, Expr::Star) {
                out.extend(self.below.iter().cloned());
                continue;
            }
            let mut vs = Vec::new();
            super::cypher::expr_vars(&it.expr, &mut vs);
            if vs.iter().all(|v| self.below.contains(v)) {
                out.push(super::cypher::item_name(it));
            }
        }
        out
    }

    fn sees_writes(&self, patterns: &[Pattern], scope: &[String]) -> bool {
        if self.labels.is_empty() && self.types.is_empty() {
            return false;
        }
        let mut bound: Vec<String> = scope.to_vec();
        for p in patterns {
            for el in &p.elems {
                match el {
                    PatElem::Node(n) => {
                        if n.var.as_ref().is_some_and(|v| bound.contains(v)) {
                            continue;
                        }
                        let groups = super::cypher::label_groups(n);
                        let sees = if groups.is_empty() {
                            !self.labels.is_empty()
                        } else {
                            groups.iter().flatten().any(|l| self.labels.contains(l))
                        };
                        if sees {
                            return true;
                        }
                        if let Some(v) = &n.var {
                            bound.push(v.clone());
                        }
                    }
                    PatElem::Rel(r) => {
                        if r.var.as_ref().is_some_and(|v| bound.contains(v)) {
                            continue;
                        }
                        let sees = if r.types.is_empty() { !self.types.is_empty() } else { r.types.iter().any(|t| self.types.contains(t)) };
                        if sees {
                            return true;
                        }
                        if let Some(v) = &r.var {
                            bound.push(v.clone());
                        }
                    }
                }
            }
        }
        false
    }

    fn after(&mut self, c: &Clause, scope: &[String], h: &Hierarchy) {
        match c {
            Clause::Set(items) | Clause::Remove(items) if !has_label_op(items) => {
                self.set = true;
                for v in scope {
                    if !self.below.contains(v) {
                        self.below.push(v.clone());
                    }
                }
            }
            Clause::With { proj, .. } if self.set => self.below = self.project(proj),
            Clause::Create(ps) => {
                let mut bound = scope.to_vec();
                for p in ps {
                    self.wrote(p, &mut bound, h);
                }
            }
            Clause::Merge { pattern, .. } => self.wrote(pattern, &mut scope.to_vec(), h),
            _ => {}
        }
    }

    /// What a CREATE / MERGE pattern may write: its new nodes' labels, its
    /// relationships' types.
    fn wrote(&mut self, p: &Pattern, bound: &mut Vec<String>, h: &Hierarchy) {
        for el in &p.elems {
            match el {
                PatElem::Node(n) => {
                    if n.var.as_ref().is_some_and(|v| bound.contains(v)) {
                        continue;
                    }
                    let ls: Vec<String> =
                        if n.labels.is_empty() { vec![String::new()] } else { n.labels.iter().flat_map(|l| h.up(l)).collect() };
                    for l in ls {
                        if !self.labels.contains(&l) {
                            self.labels.push(l);
                        }
                    }
                    if let Some(v) = &n.var {
                        bound.push(v.clone());
                    }
                }
                PatElem::Rel(r) => {
                    for t in &r.types {
                        if !self.types.contains(t) {
                            self.types.push(t.clone());
                        }
                    }
                }
            }
        }
    }
}

/// The property a MERGE that is cut for N1 marks what it created with.
const MARK: &str = "__og_c";

/// N1 (and #7): a MERGE whose ON CREATE / ON MATCH items read what it merges
/// fails in AGE 1.8.0 — `MERGE (t:T {k: 1}) ON MATCH SET t.hits =
/// coalesce(t.hits, 0) + 1` with "attribute 1 of type record has wrong type"
/// (Table has type agtype, but query expects vertex), after another clause
/// (`WITH 1 AS i MERGE (t:T {k: i}) ON MATCH SET t.hits = t.hits + 1`) and for a
/// merged relationship (`ON MATCH SET e.g = coalesce(e.g, true)`) with "binary
/// container must be a VLE vpc". Items that do not read it work (`ON MATCH SET
/// t.hits = 5`). For such a MERGE: the element that marks a creation (the
/// pattern's first relationship, given a variable if it has none, else its
/// node) and the items, both branches, to run after it ([`Seg::MergeSets`]).
/// `None`: leave the MERGE as it is.
fn merge_moves(c: &Clause, scope: &[String]) -> Option<Option<String>> {
    let Clause::Merge { pattern, on_create, on_match } = c else { return None };
    if on_create.is_empty() && on_match.is_empty() || has_label_op(on_create) || has_label_op(on_match) {
        return None;
    }
    let mut merged = Vec::new();
    super::cypher::pattern_vars(pattern, &mut merged);
    merged.retain(|v| !scope.contains(v));
    let reads = on_create.iter().chain(on_match).any(|it| match it {
        SetOp::Prop { value, .. } | SetOp::Map { value, .. } => reads_any(value, &merged),
        SetOp::Label { .. } => false,
    });
    if !reads {
        return None;
    }
    // the marker: Some(var) of the first relationship or the node; None: a
    // relationship without a variable, to be given one
    for el in &pattern.elems {
        if let PatElem::Rel(r) = el {
            return Some(r.var.clone());
        }
    }
    match pattern.elems.first() {
        Some(PatElem::Node(n)) if n.var.is_some() => Some(n.var.clone()),
        _ => None,
    }
}

/// Has the filter a part (of its ANDs) that reads, of the variables it can
/// see (`seen`), only `old` ones, or none — one the planner may put below the
/// clauses before it? (Variables of its own, a pattern's in `EXISTS { }`, do
/// not keep it up.)
fn filters_old_rows(w: &Expr, old: &[String], seen: &[String]) -> bool {
    match w {
        Expr::Binary(BinOp::And, a, b) => filters_old_rows(a, old, seen) || filters_old_rows(b, old, seen),
        e => {
            let mut vs = Vec::new();
            super::cypher::expr_vars(e, &mut vs);
            vs.iter().filter(|v| seen.contains(v)).all(|v| old.contains(v))
        }
    }
}

/// Does `e` read any of `names`?
fn reads_any(e: &Expr, names: &[String]) -> bool {
    let mut vs = Vec::new();
    super::cypher::expr_vars(e, &mut vs);
    vs.iter().any(|v| names.contains(v))
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
    // what the part being put together has done (N3, N4); a new one at every cut
    let mut cut = Cut::default();
    let flush = |cur: &mut Vec<Clause>, segs: &mut Vec<Seg>, scope: &[String], cut: &mut Cut| {
        if !cur.is_empty() {
            segs.push(Seg::Clauses(std::mem::take(cur), scope.to_vec()));
        }
        *cut = Cut::default();
    };
    let mut i = 0;
    while i < q.clauses.len() {
        let c = &q.clauses[i];
        match c {
            Clause::Foreach { var, list, body } => {
                if body.iter().any(|b| matches!(b, Clause::Foreach { .. }) || matches!(b, Clause::Set(x) | Clause::Remove(x) if has_label_op(x))) {
                    return Err(super::cypher::unsupported("FOREACH inside FOREACH, or a label change inside FOREACH"));
                }
                flush(&mut cur, &mut segs, &scope, &mut cut);
                segs.push(Seg::Foreach { var: var.clone(), list: list.clone(), body: body.clone() });
            }
            Clause::Subquery { scope: sc, body, optional } => {
                if must_cut(body) {
                    return Err(super::cypher::unsupported("FOREACH, CALL { … } or a label change inside CALL { … }"));
                }
                flush(&mut cur, &mut segs, &scope, &mut cut);
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
                            flush(&mut cur, &mut segs, &scope, &mut cut);
                        }
                        cur.push(if add { Clause::Set(rest) } else { Clause::Remove(rest) });
                    }
                    j += 1;
                }
                flush(&mut cur, &mut segs, &scope, &mut cut);
                segs.push(Seg::Relabel(ops));
                i = j;
                continue;
            }
            Clause::Merge { on_create, on_match, .. } if has_label_op(on_create) || has_label_op(on_match) => {
                return Err(super::cypher::unsupported("a label set in ON CREATE / ON MATCH"));
            }
            other => {
                if cur.last().is_some_and(|l| merge_then_set(l, other)) || (!cur.is_empty() && cut.before(other, &scope)) {
                    flush(&mut cur, &mut segs, &scope, &mut cut);
                }
                match (other, merge_moves(other, &scope)) {
                    (Clause::Merge { pattern, on_create, on_match }, Some(marker)) => {
                        // N1: the MERGE marks what it creates; its items run after it.
                        let mut pattern = pattern.clone();
                        let marker = match marker {
                            Some(v) => v,
                            None => {
                                let v = p.fresh("m");
                                if let Some(PatElem::Rel(r)) = pattern.elems.iter_mut().find(|e| matches!(e, PatElem::Rel(_))) {
                                    r.var = Some(v.clone());
                                }
                                v
                            }
                        };
                        let mark = SetOp::Prop { var: marker.clone(), prop: MARK.into(), value: Expr::Lit(Lit::Bool(true)) };
                        let merge = Clause::Merge { pattern, on_create: vec![mark], on_match: Vec::new() };
                        let after = scope_after(&scope, &merge);
                        cur.push(merge);
                        flush(&mut cur, &mut segs, &after, &mut cut);
                        segs.push(Seg::MergeSets { marker, on_create: on_create.clone(), on_match: on_match.clone() });
                        scope = after;
                        i += 1;
                        continue;
                    }
                    _ => {
                        cut.after(other, &scope, &p.hier);
                        cur.push(other.clone());
                    }
                }
            }
        }
        scope = scope_after(&scope, c);
        i += 1;
    }
    flush(&mut cur, &mut segs, &scope, &mut cut);
    Ok(Steps { ncols, segs, now: p.now.clone(), hier: p.hier.clone() })
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

/// The key of `$__og_rows` that says a row is the first to reach the element
/// a cut MERGE marks ([`Seg::MergeSets`]).
const FIRST: &str = "__og_first";

/// How many of the clauses, from the start, are SET / REMOVE of properties.
fn leading_sets(cs: &[Clause]) -> usize {
    cs.iter().take_while(|c| matches!(c, Clause::Set(items) | Clause::Remove(items) if !has_label_op(items))).count()
}

fn set_items(c: &Clause) -> &[SetOp] {
    match c {
        Clause::Set(items) | Clause::Remove(items) => items,
        _ => &[],
    }
}

/// The variables the items write.
fn set_targets<'a>(items: impl IntoIterator<Item = &'a SetOp>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for it in items {
        let v = match it {
            SetOp::Prop { var, .. } | SetOp::Map { var, .. } | SetOp::Label { var, .. } => var,
        };
        if !out.contains(v) {
            out.push(v.clone());
        }
    }
    out
}

/// The rows (their indexes) in waves for a step that writes `targets`: no
/// element twice in one wave, and each element's rows in their order from one
/// wave to the next. One wave when every row writes elements of its own.
///
/// N2: AGE 1.8.0 loses a write when two rows of one statement update the same
/// element. Each row's SET starts from the element as the row read it, before
/// the statement (the scans below a SET run with the statement's command id
/// lowered — `cypher_set.c` `exec_cypher_set`), so the second row's write
/// replaces the first's; and a SET that ends the statement updates under one
/// command id, where `update_entity_tuple` skips a tuple that command already
/// updated (`TM_SelfModified`). `UNWIND [1, 1, 1] AS i MERGE (t:U {k: i}) ON
/// MATCH SET t.hits = coalesce(t.hits, 0) + 1` stores 1 (Neo4j 2).
fn waves(rows: &[Row], targets: &[String]) -> Vec<Vec<usize>> {
    let mut last: HashMap<i64, usize> = HashMap::new();
    let mut out: Vec<Vec<usize>> = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let mut ids: Vec<i64> = Vec::new();
        for id in targets.iter().filter_map(|t| r.get(t).and_then(Ag::element_id)) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        let w = ids.iter().filter_map(|id| last.get(id)).map(|w| w + 1).max().unwrap_or(0);
        for id in ids {
            last.insert(id, w);
        }
        if out.len() <= w {
            out.resize(w + 1, Vec::new());
        }
        out[w].push(i);
    }
    out
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
    let mut hier = steps.hier.clone();
    for (k, seg) in steps.segs.iter().enumerate() {
        let mut printer = Printer::new(Some(&pmap), steps.now.clone()).with_hierarchy(hier.clone());
        let ks = kinds(&rows, &scope)?;
        let pre = if first { None } else { Some(prelude(&ks)) };
        match seg {
            Seg::Clauses(cs, after) => {
                let mut cs: &[Clause] = cs;
                // N2: a part that starts with SET / REMOVE on rows that hold the
                // same element more than once runs those once per wave first.
                let lead = if first { 0 } else { leading_sets(cs) };
                if lead > 0 {
                    let ws = waves(&rows, &set_targets(cs[..lead].iter().flat_map(set_items)));
                    if ws.len() > 1 {
                        for w in &ws {
                            let sub: Vec<Row> = w.iter().map(|&i| rows[i].clone()).collect();
                            let mut text = prelude(&ks);
                            let mut sc = scope.clone();
                            for c in &cs[..lead] {
                                text.push(' ');
                                text.push_str(&printer.clause(c, &sc).map_err(Refusal::syntax)?);
                                sc = scope_after(&sc, c);
                            }
                            let p = with_param(&merged(params, &printer.extra), ROWS, rows_param(&sub, &ks));
                            x.cypher(&text, 0, &p)?;
                        }
                        cs = &cs[lead..];
                        if cs.is_empty() {
                            // only the writes: the rows go on as they were
                            if k == last {
                                return Ok((Vec::new(), adjust));
                            }
                            scope = after.clone();
                            continue;
                        }
                    }
                }
                let rows_json = rows_param(&rows, &ks);
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
                let rows_json = rows_param(&rows, &ks);
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
            Seg::MergeSets { marker, on_create, on_match } => {
                // The first row that reaches an element is the one that may
                // have created it; it did when the element carries the mark.
                let mut seen = HashSet::new();
                let first_of: Vec<bool> =
                    rows.iter().map(|r| r.get(marker).and_then(Ag::element_id).is_some_and(|id| seen.insert(id))).collect();
                let mut targets = vec![marker.clone()];
                targets.extend(set_targets(on_create.iter().chain(on_match)));
                let carry: Vec<String> = ks.iter().map(|(v, _, _)| super::cypher::ident(v)).collect();
                let mk = super::cypher::ident(marker);
                // N2: rows that reach the same element run one wave after
                // another, so each sees what the one before it wrote.
                for w in waves(&rows, &targets) {
                    if !on_match.is_empty() {
                        let sub: Vec<Row> = w.iter().map(|&i| rows[i].clone()).collect();
                        let mut rj = rows_param(&sub, &ks);
                        if let Json::Array(xs) = &mut rj {
                            for (x, &i) in xs.iter_mut().zip(&w) {
                                if let Json::Object(m) = x {
                                    m.insert(FIRST.into(), Json::Bool(first_of[i]));
                                }
                            }
                        }
                        let text = format!(
                            "{} WITH {ROW}, {} WHERE (NOT {ROW}.{FIRST}) OR ({mk}.{MARK} IS NULL) {}",
                            prelude(&ks),
                            carry.join(", "),
                            printer.clause(&Clause::Set(on_match.clone()), &scope).map_err(Refusal::syntax)?
                        );
                        x.cypher(&text, 0, &with_param(&merged(params, &printer.extra), ROWS, rj))?;
                    }
                    let creators: Vec<Row> = w.iter().filter(|&&i| first_of[i]).map(|&i| rows[i].clone()).collect();
                    if !creators.is_empty() {
                        let mut items = on_create.clone();
                        items.push(SetOp::Prop { var: marker.clone(), prop: MARK.into(), value: Expr::Lit(Lit::Null) });
                        let text = format!(
                            "{} WITH {} WHERE {mk}.{MARK} IS NOT NULL {} RETURN count(*)",
                            prelude(&ks),
                            carry.join(", "),
                            printer.clause(&Clause::Set(items), &scope).map_err(Refusal::syntax)?
                        );
                        let got = x.cypher(&text, 1, &with_param(&merged(params, &printer.extra), ROWS, rows_param(&creators, &ks)))?;
                        let n = match got.first().and_then(|r| r.first()) {
                            Some(Ag::Int(n)) => *n,
                            _ => 0,
                        };
                        // the mark set and taken off again: not the client's
                        // properties (a MERGE with ON CREATE items updated the
                        // element once anyway)
                        adjust.properties_set -= n * if on_create.is_empty() { 2 } else { 1 };
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
                        // Its labels are its table's and those that table
                        // inherits (`hierarchy.rs`): which table it moves to,
                        // and the inheritance that table gets first.
                        let stored = if label.is_empty() || label == "_ag_label_vertex" { None } else { Some(label.as_str()) };
                        let mut schema = Schema::default();
                        let (target, added, removed) = match hier.relabel(var, stored, add, remove, &mut schema)? {
                            Move::Stay => continue,
                            Move::To { label: to, added, removed } => (to.unwrap_or_else(|| "_ag_label_vertex".into()), added, removed),
                        };
                        hierarchy::apply(x, &schema)?;
                        if target == label {
                            continue;
                        }
                        let new_id = relabel(x, &graph, id, &target, &mut hier)?;
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
/// rows were pointed at it. A table the hierarchy does not know of yet is
/// looked up, and made if missing.
///
/// The id changes: AGE's graphid carries the label's number, so a vertex in
/// another table is another id — an AGE limit (STUDIO G6). An element id a
/// client kept from before the change no longer finds it.
fn relabel(x: &mut dyn Exec, graph: &str, id: i64, target: &str, hier: &mut Hierarchy) -> Result<(i64, i64), Refusal> {
    let g = sql_ident(graph);
    if target != "_ag_label_vertex" && !hier.vertex.contains(target) {
        let have = x.sql(&format!(
            "SELECT l.kind::text FROM ag_catalog.ag_label l JOIN ag_catalog.ag_graph g ON g.graphid = l.graph \
             WHERE g.name = {} AND l.name = {}",
            sql_lit(graph),
            sql_lit(target)
        ))?;
        match have.first().and_then(|r| r[0].clone()) {
            None => {
                x.sql(&hierarchy::create_vlabel_sql(graph, target))?;
            }
            Some(k) if k == "v" => {}
            Some(_) => {
                return Err(Refusal::syntax(format!("'{target}' is a relationship type in graph '{graph}', not a label")));
            }
        }
        hier.vertex.insert(target.to_string());
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
        steps_in(q, Hierarchy::default())
    }

    fn steps_in(q: &str, h: Hierarchy) -> Steps {
        let ast = cypher_syntax::parse(q).unwrap();
        assert!(needed(&ast, &h));
        let mut p = Printer::new(None, "2026-11-18T00:00:00+00:00".into()).with_hierarchy(h);
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
        assert!(f.asked[2].starts_with("SELECT ag_catalog.create_vlabel('g'::cstring, 'BpmnProcess'::cstring) WHERE NOT EXISTS"), "{:?}", f.asked);
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

    fn not_cut(q: &str) -> bool {
        !needed(&cypher_syntax::parse(q).unwrap(), &Hierarchy::default())
    }

    #[test]
    fn a_filter_after_set_runs_in_the_next_statement() {
        // N4
        let s = steps("MATCH (s:V) SET s.v = s.k * 10 WITH s WHERE s.v > 10 RETURN collect(s.v) AS vs");
        let mut f = Fake { asked: vec![], answers: vec![vec![vec![v(1, "V")], vec![v(2, "V")], vec![v(3, "V")]], vec![vec![Ag::List(vec![Ag::Int(20), Ag::Int(30)])]]] };
        let (rows, _) = run(&mut f, &s, &Json::Object(Map::new())).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(f.asked[0], "MATCH (s:V) SET s.v = (s.k * 10) RETURN s | rows=[{\"__i\":0}]");
        assert!(f.asked[1].starts_with("UNWIND $__og_rows AS __og_row OPTIONAL MATCH (s:V) WHERE id(s) = __og_row.s WITH __og_row, s WITH s WHERE s.v > 10 RETURN collect(s.v) AS vs |"), "{}", f.asked[1]);
        assert!(!not_cut("MATCH (s) REMOVE s.x MATCH (t:T) WHERE s.k > 1 AND t.k = s.k RETURN t"));
        assert!(!not_cut("MATCH (s) SET s.x = 1 WITH s OPTIONAL MATCH (t:T) WHERE $flag RETURN t"));
        // a filter on the new variables only cannot go below the SET (Robo's shape)
        assert!(not_cut("MERGE (c:Command {id: $id}) SET c.name = $n WITH c OPTIONAL MATCH (a:Aggregate {id: $a}) WHERE a IS NOT NULL MERGE (a)-[:HAS_COMMAND]->(c) RETURN c.id AS id"));
        assert!(not_cut("MATCH (s) REMOVE s.x MATCH (t:T) WHERE t.k = s.k RETURN t"));
        assert!(not_cut("MATCH (g:G) SET g.x = 1 WITH g, $refs AS refs UNWIND refs AS ref WITH g, ref WHERE ref.id IS NOT NULL RETURN g"));
        assert!(!not_cut("MATCH (s:V) SET s.v = 1 WITH s, s.k AS k WHERE k > 1 RETURN s"));
        assert!(!not_cut("MATCH (s:V) SET s.v = 1 WITH s, $refs AS refs WITH s, refs WHERE size(refs) > 0 RETURN s"));
        assert!(!not_cut("MATCH (s:V) SET s.v = 1 WITH s WHERE EXISTS { MATCH (s)-[:R]->(x) WHERE x.k = 1 } RETURN s"));
        // no filter after it, or the filter before it: one statement
        assert!(not_cut("MATCH (s:V) SET s.u = 1 WITH s, s.k AS k RETURN k"));
        assert!(not_cut("MATCH (s:V) WHERE s.k > 1 SET s.u = 1 RETURN s"));
    }

    #[test]
    fn a_match_after_a_write_that_can_see_it_runs_in_the_next_statement() {
        // N3
        let s = steps("UNWIND [1, 2, 3] AS i CREATE (s:V {k: i}) WITH count(s) AS made MATCH (t:V) RETURN made, count(t) AS c");
        let mut f = Fake { asked: vec![], answers: vec![vec![vec![Ag::Int(3)]], vec![vec![Ag::Int(3), Ag::Int(3)]]] };
        let (rows, _) = run(&mut f, &s, &Json::Object(Map::new())).unwrap();
        assert_eq!(rows, vec![vec![Ag::Int(3), Ag::Int(3)]]);
        assert_eq!(f.asked[0], "UNWIND [1, 2, 3] AS i CREATE (s:V {k: i}) WITH count(s) AS made RETURN made | rows=[{\"__i\":0}]");
        assert!(f.asked[1].contains("WITH __og_row, __og_row.made AS made MATCH (t:V) RETURN made, count(t) AS c |"), "{}", f.asked[1]);
        // what was written: a label, an unlabelled node, a relationship type
        assert!(!not_cut("CREATE (a:X) WITH a OPTIONAL MATCH (b) RETURN count(b)"));
        assert!(!not_cut("MATCH (a:A), (b:B) CREATE (a)-[:R]->(b) WITH a MATCH (a)-[r:R]->() RETURN count(r)"));
        assert!(!not_cut("MERGE (a:X {id: 1}) WITH a UNWIND [2, 3] AS k MATCH (b:X {id: k}) RETURN b"));
        // labels and types not written stay in the statement (Robo's MERGE … WITH … MATCH)
        assert!(not_cut("MERGE (a:X {id: $a}) WITH a MATCH (b:Y {id: $b}) MERGE (a)-[:R]->(b)"));
        assert!(not_cut("MATCH (a:A), (b:B) CREATE (a)-[:R]->(b) WITH a MATCH (a)-[:S]->(c:C) RETURN c"));
        assert!(not_cut("CREATE (a:X) WITH a MATCH (a)-[:S]->(c:C) RETURN c"));
    }

    #[test]
    fn merge_items_that_read_what_it_merged_run_after_it() {
        // N1
        let s = steps("MERGE (t:T {k: 1}) ON MATCH SET t.hits = coalesce(t.hits, 0) + 1");
        let mut f = Fake { asked: vec![], answers: vec![vec![vec![v(5, "T")]], vec![], vec![vec![Ag::Int(0)]]] };
        let (rows, adj) = run(&mut f, &s, &Json::Object(Map::new())).unwrap();
        assert!(rows.is_empty());
        assert_eq!(f.asked.len(), 3, "{:?}", f.asked);
        assert_eq!(f.asked[0], "MERGE (t:T {k: 1}) ON CREATE SET t.__og_c = true RETURN t | rows=[{\"__i\":0}]");
        assert_eq!(f.asked[1], "UNWIND $__og_rows AS __og_row OPTIONAL MATCH (t:T) WHERE id(t) = __og_row.t WITH __og_row, t \
            WITH __og_row, t WHERE (NOT __og_row.__og_first) OR (t.__og_c IS NULL) SET t.hits = (coalesce(t.hits, 0) + 1) \
            | rows=[{\"__i\":0,\"__og_first\":true,\"t\":5}]");
        assert_eq!(f.asked[2], "UNWIND $__og_rows AS __og_row OPTIONAL MATCH (t:T) WHERE id(t) = __og_row.t WITH __og_row, t \
            WITH t WHERE t.__og_c IS NOT NULL SET t.__og_c = null RETURN count(*) | rows=[{\"__i\":0,\"t\":5}]");
        assert_eq!(adj.properties_set, 0);
        // a merged relationship (#7), given a variable to mark; ON CREATE items move too
        let s = steps("MATCH (a:A) MERGE (a)-[:R]->(b:B {k: 1}) ON CREATE SET b.n = 1 ON MATCH SET b.n = b.n + 1 RETURN b.n AS n");
        let mut f = Fake { asked: vec![], answers: vec![vec![vec![v(1, "A"), Ag::Edge { id: 9, label: "R".into(), start: 1, end: 2, props: vec![] }, v(2, "B")]], vec![], vec![vec![Ag::Int(1)]], vec![vec![Ag::Int(1)]]] };
        let (rows, adj) = run(&mut f, &s, &Json::Object(Map::new())).unwrap();
        assert_eq!(rows, vec![vec![Ag::Int(1)]]);
        assert_eq!(f.asked[0], "MATCH (a:A) MERGE (a)-[__og_m1:R]->(b:B {k: 1}) ON CREATE SET __og_m1.__og_c = true RETURN a, __og_m1, b | rows=[{\"__i\":0}]");
        assert!(f.asked[1].contains("WHERE (NOT __og_row.__og_first) OR (__og_m1.__og_c IS NULL) SET b.n = (b.n + 1) |"), "{}", f.asked[1]);
        assert!(f.asked[2].contains("WHERE __og_m1.__og_c IS NOT NULL SET b.n = 1, __og_m1.__og_c = null RETURN count(*)"), "{}", f.asked[2]);
        assert!(f.asked[3].contains("RETURN b.n AS n"), "{}", f.asked[3]);
        assert_eq!(adj.properties_set, -1);
        // items that do not read it: the MERGE as it is
        assert!(not_cut("MERGE (t:T {k: 1}) ON MATCH SET t.hits = 5 ON CREATE SET t.at = $now"));
        assert!(not_cut("MATCH (a) MERGE (t:T {k: 1}) ON MATCH SET t.x = a.x"));
        assert!(not_cut("MATCH (a:A), (b:B) MERGE (a)-[:R]->(b) ON MATCH SET b.n = b.n + 1"));
        assert!(!not_cut("MATCH (a:A), (b:B) MERGE (a)-[e:R]->(b) ON MATCH SET e.g = coalesce(e.g, true)"));
    }

    #[test]
    fn rows_that_reach_the_same_element_write_one_after_another() {
        // N2: the three rows merge one vertex; each ON MATCH sees the one before
        let s = steps("UNWIND [1, 1, 1] AS i MERGE (t:U {k: i}) ON MATCH SET t.hits = coalesce(t.hits, 0) + 1 RETURN t.hits AS h");
        let row = vec![Ag::Int(1), v(7, "U")];
        let mut f = Fake {
            asked: vec![],
            answers: vec![vec![row.clone(), row.clone(), row], vec![], vec![vec![Ag::Int(1)]], vec![], vec![], vec![vec![Ag::Int(2)], vec![Ag::Int(2)], vec![Ag::Int(2)]]],
        };
        let (rows, _) = run(&mut f, &s, &Json::Object(Map::new())).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(f.asked.len(), 6, "{:#?}", f.asked);
        // wave 1: the first row, matched (no) then created (yes); waves 2, 3: matched
        assert!(f.asked[1].contains("(NOT __og_row.__og_first)") && f.asked[1].ends_with(r#"rows=[{"__i":0,"__og_first":true,"i":1,"t":7}]"#), "{}", f.asked[1]);
        assert!(f.asked[2].contains("t.__og_c IS NOT NULL"), "{}", f.asked[2]);
        assert!(f.asked[3].ends_with(r#"rows=[{"__i":0,"__og_first":false,"i":1,"t":7}]"#), "{}", f.asked[3]);
        assert!(f.asked[4].ends_with(r#"rows=[{"__i":0,"__og_first":false,"i":1,"t":7}]"#), "{}", f.asked[4]);
        assert!(f.asked[5].contains("RETURN t.hits AS h"), "{}", f.asked[5]);
        // distinct elements: one wave
        let s = steps("UNWIND [1, 2] AS i MERGE (t:U {k: i}) ON MATCH SET t.hits = t.hits + 1");
        let mut f = Fake { asked: vec![], answers: vec![vec![vec![Ag::Int(1), v(7, "U")], vec![Ag::Int(2), v(8, "U")]], vec![], vec![vec![Ag::Int(2)]]] };
        run(&mut f, &s, &Json::Object(Map::new())).unwrap();
        assert_eq!(f.asked.len(), 3, "{:#?}", f.asked);
        // a SET after the cut, on rows that hold one element twice
        let s = steps("UNWIND $rows AS r MERGE (t:U {k: r.k}) ON CREATE SET t.c = 1 SET t.n = coalesce(t.n, 0) + r.d RETURN t.n AS n");
        let r1 = vec![Ag::Map(vec![("k".into(), Ag::Int(1)), ("d".into(), Ag::Int(2))]), v(7, "U")];
        let r2 = vec![Ag::Map(vec![("k".into(), Ag::Int(1)), ("d".into(), Ag::Int(3))]), v(7, "U")];
        let mut f = Fake { asked: vec![], answers: vec![vec![r1, r2], vec![], vec![], vec![vec![Ag::Int(5)], vec![Ag::Int(5)]]] };
        let (rows, _) = run(&mut f, &s, &serde_json::json!({"rows": []})).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(f.asked.len(), 4, "{:#?}", f.asked);
        assert!(f.asked[1].contains("SET t.n = (coalesce(t.n, 0) + r.d) |") && f.asked[1].contains(r#""d":2"#) && !f.asked[1].contains(r#""d":3"#), "{}", f.asked[1]);
        assert!(f.asked[2].contains(r#""d":3"#), "{}", f.asked[2]);
        // then the rest of the part, on all the rows
        assert!(f.asked[3].contains("RETURN t.n AS n |") && f.asked[3].contains(r#""d":2"#) && f.asked[3].contains(r#""d":3"#), "{}", f.asked[3]);
        assert_eq!(waves(&[], &["t".into()]), Vec::<Vec<usize>>::new());
    }

    #[test]
    fn a_label_change_keeps_the_labels_the_table_inherits() {
        // G6: the class renamed — the new table inherits the old one's parent
        // first (DDL only), then the vertex moves as before
        let h = Hierarchy::of(&[("Customer", "_Entity")], &[]);
        let s = steps_in("MATCH (n:_Entity:Customer) WHERE n.name = 'a' REMOVE n:Customer SET n:Client RETURN count(n) AS c", h.clone());
        let mut f = Fake { asked: vec![], answers: vec![vec![vec![v(844424930131969, "Customer")]], vec![vec![Ag::Int(1)]]] };
        let (rows, adj) = run(&mut f, &s, &Json::Object(Map::new())).unwrap();
        assert_eq!(rows, vec![vec![Ag::Int(1)]]);
        assert_eq!(f.asked[0], "MATCH (n:Customer) WHERE n.name = 'a' RETURN n | rows=[{\"__i\":0}]");
        assert!(f.asked[1].starts_with("SELECT ag_catalog.create_vlabel('g'::cstring, 'Client'::cstring)"), "{:#?}", f.asked);
        assert_eq!(f.asked[2], "ALTER TABLE \"g\".\"Client\" INHERIT \"g\".\"_Entity\"");
        assert!(f.asked[3].contains("INSERT INTO \"g\".\"Client\" (properties)"), "{}", f.asked[3]);
        assert!(f.asked[5].contains("OPTIONAL MATCH (n:Client) WHERE id(n) = __og_row.n"), "{}", f.asked[5]);
        assert_eq!(f.asked.len(), 6, "{:#?}", f.asked);
        assert_eq!((adj.nodes_created, adj.nodes_deleted, adj.labels_added, adj.labels_removed), (-1, -1, 0, 1));
        // a label it has through its table: nothing happens, nothing is counted
        let s = steps_in("MATCH (n:Customer) SET n:_Entity RETURN count(n) AS c", h.clone());
        let mut f = Fake { asked: vec![], answers: vec![vec![vec![v(7, "Customer")]], vec![vec![Ag::Int(1)]]] };
        let (_, adj) = run(&mut f, &s, &Json::Object(Map::new())).unwrap();
        assert_eq!(f.asked.len(), 2, "{:#?}", f.asked);
        assert_eq!(adj, Stats::default());
        // and it cannot be taken off one node
        let s = steps_in("MATCH (n:Customer) REMOVE n:_Entity RETURN n", h.clone());
        let mut f = Fake { asked: vec![], answers: vec![vec![vec![v(7, "Customer")]]] };
        let e = run(&mut f, &s, &Json::Object(Map::new())).err().unwrap();
        assert_eq!(e.code, "Neo.ClientError.Statement.SemanticError");
        assert!(e.message.starts_with("REMOVE n:_Entity is refused on the AGE backend"), "{}", e.message);
        // N3: a MATCH of the parent sees what a write of the child wrote
        let q = cypher_syntax::parse("CREATE (n:Customer {k: 1}) WITH n MATCH (m:_Entity) RETURN count(m) AS c").unwrap();
        assert!(needed(&q, &h));
        assert!(!needed(&q, &Hierarchy::default()));
    }

    #[test]
    fn two_labels_are_refused() {
        let s = steps("MATCH (n:A) SET n:B RETURN n");
        let mut f = Fake { asked: vec![], answers: vec![vec![vec![v(1, "A")]]] };
        let e = run(&mut f, &s, &Json::Null).err().unwrap();
        assert!(e.message.contains("one label"), "{}", e.message);
    }
}
