//! Long variable-length relationships the gateway walks itself.
//!
//! AGE's VLE filters a variable-length relationship by one type at most and
//! enumerates every trail of what is left; Robo's storyboard walk
//! (`-[:EMITS|TRIGGERS|…10 types…*1..30]-`, `<-[*1..30]-`) took the server to
//! its memory limit that way (killed by the kernel, every connection lost).
//! Such a relationship — more than [`MAX_UNTYPED_HOPS`] hops, not exactly one
//! type — is answered here when only *which nodes it reaches* matters, which
//! is how all three of Robo's are used:
//!
//! - **A.** inside `EXISTS { … }` (or a pattern predicate), both ends already
//!   bound: `EXISTS { MATCH (c)-[:A|B*1..30]-(u) }`;
//! - **B.** the only pattern of a `[OPTIONAL] MATCH (s …)-[…*1..30]-(t …)`
//!   with `t` new, when what follows cannot tell one row per trail from one row
//!   per end node: the next projection is `DISTINCT`, or has only `DISTINCT`
//!   aggregates (and `min`/`max`), or has no aggregate and `LIMIT 1` (the
//!   clauses in between may only read). Neo4j's planner makes the same call
//!   (pruning var-length expand).
//!
//! Anything else with such a relationship — a relationship or path variable,
//! a count of the matches, rows per trail — is still refused: those need the
//! trails themselves.
//!
//! How it runs (one transaction):
//! 1. **Seeds**: the clauses before the site, then `RETURN DISTINCT` the start
//!    node's id (the end bound earlier, else the one with properties).
//! 2. **Walk**: breadth first from every seed at once, one SQL statement per
//!    hop on the label tables' `start_id`/`end_id` (the types' tables, or
//!    `_ag_label_edge` for any type), with a visited set per seed.
//! 3. **Statement**: the original with the site replaced by a test against
//!    `$__og_reachN`, a map from each seed's id to the ids it reaches:
//!    `id(coalesce(t)) IN coalesce($__og_reachN[toString(id(coalesce(s)))], [])`.
//!
//! The answer is Neo4j's (trails, not walks): for a node other than the
//! start, a trail of 1..n hops reaches it exactly when the shortest path is at
//! most n hops (a shortest walk repeats no relationship). The start itself is
//! reached when a closed trail through it has at most n hops: a directed cycle
//! through it, or, undirected, two tree paths with different first
//! relationships joined by one more relationship (the shortest cycle through a
//! vertex, from the same breadth-first tree). A lower bound above 1 is refused.

use super::agtype::Ag;
use super::cypher::{and_all, is_write, scope_after, scope_of, unsupported, Printer, MAX_UNTYPED_HOPS};
use super::split::Exec;
use super::{sql_lit, Refusal};
use cypher_syntax::ast::*;
use serde_json::{Map, Value as Json};
use std::collections::{HashMap, HashSet};

/// The function the printer writes out as its one string argument, as is.
pub const RAW: &str = "__og_raw";

fn raw(text: String) -> Expr {
    Expr::Func { name: RAW.into(), args: vec![Expr::Lit(Lit::Str(text))], distinct: false }
}

/// A variable-length relationship AGE cannot be given.
pub fn long(r: &RelPat) -> bool {
    r.range.is_some_and(|(_, hi)| r.types.len() != 1 && hi > MAX_UNTYPED_HOPS)
}

fn pattern_long(p: &Pattern) -> bool {
    p.elems.iter().any(|e| matches!(e, PatElem::Rel(r) if long(r)))
}

/// The walk, as planned.
#[derive(Debug)]
pub struct Reach {
    /// AGE Cypher: the seeds' ids, one column.
    pub seed: String,
    /// The types walked; empty: any.
    pub types: Vec<String>,
    /// Seen from the start node.
    pub dir: Dir,
    pub lo: u32,
    pub hi: u32,
    /// The parameter the statement reads the reached ids from.
    pub param: String,
    /// The statement, AGE Cypher.
    pub text: String,
    pub ncols: usize,
}

// ------------------------------------------------------------------ finding

/// Long relationships in an expression (subqueries' patterns).
fn expr_longs(e: &Expr, n: &mut usize) {
    if let Expr::Subquery { patterns, .. } = e {
        *n += patterns.iter().filter(|p| pattern_long(p)).count();
    }
    let mut go = |x: &Expr| expr_longs(x, n);
    match e {
        Expr::Subquery { where_, project, .. } => {
            if let Some(w) = where_ {
                go(w);
            }
            if let Some(p) = project {
                go(p);
            }
        }
        Expr::Prop(b, _) | Expr::Not(b) | Expr::Neg(b) | Expr::IsNull(b, _) => go(b),
        Expr::Binary(_, a, b) => {
            go(a);
            go(b);
        }
        Expr::Func { args, .. } | Expr::List(args) => args.iter().for_each(go),
        Expr::Map(kv) => kv.iter().for_each(|(_, v)| go(v)),
        Expr::Case { operand, whens, else_ } => {
            if let Some(o) = operand {
                go(o);
            }
            for (a, b) in whens {
                go(a);
                go(b);
            }
            if let Some(x) = else_ {
                go(x);
            }
        }
        Expr::ListComp { source, filter, project, .. } => {
            go(source);
            if let Some(f) = filter {
                go(f);
            }
            if let Some(p) = project {
                go(p);
            }
        }
        Expr::ListPred { source, filter, .. } => {
            go(source);
            go(filter);
        }
        Expr::MapProjection { items, .. } => {
            for i in items {
                if let MapProjItem::Entry(_, v) = i {
                    go(v);
                }
            }
        }
        Expr::Slice { list, from, to } => {
            go(list);
            if let Some(f) = from {
                go(f);
            }
            if let Some(t) = to {
                go(t);
            }
        }
        Expr::Reduce { init, list, expr, .. } => {
            go(init);
            go(list);
            go(expr);
        }
        _ => {}
    }
}

fn proj_exprs(p: &Projection) -> Vec<&Expr> {
    let mut v: Vec<&Expr> = p.items.iter().map(|i| &i.expr).collect();
    v.extend(p.order.iter().map(|o| &o.expr));
    v
}

/// Long relationships in a clause: (in its patterns, in its expressions).
fn clause_longs(c: &Clause) -> (usize, usize) {
    let mut in_exprs = 0;
    let mut in_patterns = 0;
    let mut ex = |e: &Expr| expr_longs(e, &mut in_exprs);
    match c {
        Clause::Match { patterns, where_, .. } => {
            in_patterns += patterns.iter().filter(|p| pattern_long(p)).count();
            for p in patterns {
                node_prop_exprs(p).into_iter().for_each(&mut ex);
            }
            if let Some(w) = where_ {
                ex(w);
            }
        }
        Clause::Create(patterns) => {
            in_patterns += patterns.iter().filter(|p| pattern_long(p)).count();
        }
        Clause::Merge { pattern, on_create, on_match } => {
            in_patterns += usize::from(pattern_long(pattern));
            for s in on_create.iter().chain(on_match) {
                set_exprs(s).into_iter().for_each(&mut ex);
            }
        }
        Clause::Unwind { expr, .. } => ex(expr),
        Clause::With { proj, where_ } => {
            proj_exprs(proj).into_iter().for_each(&mut ex);
            if let Some(w) = where_ {
                ex(w);
            }
        }
        Clause::Return(proj) => proj_exprs(proj).into_iter().for_each(&mut ex),
        Clause::Set(items) | Clause::Remove(items) => {
            for s in items {
                set_exprs(s).into_iter().for_each(&mut ex);
            }
        }
        Clause::Delete { exprs, .. } => exprs.iter().for_each(&mut ex),
        Clause::Foreach { list, body, .. } => {
            ex(list);
            for b in body {
                let (a, e) = clause_longs(b);
                in_patterns += a;
                in_exprs += e;
            }
        }
        Clause::Subquery { body, .. } => {
            let n = query_longs(body);
            in_patterns += n;
        }
        Clause::Call { args, .. } => args.iter().for_each(&mut ex),
        Clause::Ddl(_) => {}
    }
    (in_patterns, in_exprs)
}

fn node_prop_exprs(p: &Pattern) -> Vec<&Expr> {
    let mut v = Vec::new();
    for el in &p.elems {
        match el {
            PatElem::Node(n) => v.extend(n.props.iter().map(|(_, e)| e)),
            PatElem::Rel(r) => v.extend(r.props.iter().map(|(_, e)| e)),
        }
    }
    v
}

fn set_exprs(s: &SetOp) -> Vec<&Expr> {
    match s {
        SetOp::Prop { value, .. } | SetOp::Map { value, .. } => vec![value],
        SetOp::Label { .. } => vec![],
    }
}

fn query_longs(q: &Query) -> usize {
    let mut n: usize = q.clauses.iter().map(|c| {
        let (a, b) = clause_longs(c);
        a + b
    }).sum();
    if let Some((_, t)) = &q.union {
        n += query_longs(t);
    }
    n
}

/// Does the statement have a relationship only this module can run?
pub fn needed(q: &Query) -> bool {
    query_longs(q) > 0
}

// ------------------------------------------------------------------ planning

/// The two ends of a one-relationship pattern and the relationship.
fn three(p: &Pattern) -> Option<(&NodePat, &RelPat, &NodePat)> {
    match p.elems.as_slice() {
        [PatElem::Node(a), PatElem::Rel(r), PatElem::Node(b)] => Some((a, r, b)),
        _ => None,
    }
}

fn plain_rel(r: &RelPat) -> Result<(), String> {
    if r.var.is_some() {
        return Err("a variable on it (the gateway walks it for the nodes it reaches, not the relationships)".into());
    }
    if !r.props.is_empty() {
        return Err("properties on it".into());
    }
    match r.range {
        Some((lo, _)) if lo > 1 => Err(format!("a lower bound of {lo} (only 0 or 1)")),
        _ => Ok(()),
    }
}

fn refuse(r: &RelPat, why: &str) -> String {
    let (_, hi) = r.range.unwrap_or((1, 1));
    format!(
        "a variable-length relationship with {} and up to {hi} hops is not run on the AGE backend here: AGE filters a \
         variable-length pattern by one type only and walks every trail of the rest (*1..30 over every type took the \
         server to its memory limit), and the gateway walks it itself only for the nodes it reaches — inside \
         EXISTS {{ }} between two bound nodes, or as the only pattern of a MATCH followed by DISTINCT, DISTINCT \
         aggregates or LIMIT 1. Here: {why}. Give it one type, or at most {MAX_UNTYPED_HOPS} hops",
        if r.types.is_empty() { "no type".to_string() } else { format!("the types {}", r.types.join("|")) }
    )
}

/// The test that replaces the walk: is `t` among the ids `s` reaches?
fn reached(param: &str, s: &str, t: &str) -> Expr {
    let (s, t) = (super::cypher::ident(s), super::cypher::ident(t));
    raw(format!(
        "({s} IS NOT NULL AND {t} IS NOT NULL AND id(coalesce({t})) IN coalesce(${param}[toString(id(coalesce({s})))], []))"
    ))
}

fn seed_return(s: &str) -> Clause {
    Clause::Return(Projection {
        items: vec![ReturnItem {
            expr: raw(format!("id(coalesce({}))", super::cypher::ident(s))),
            alias: Some("__og_seed".into()),
        }],
        distinct: true,
        ..Projection::default()
    })
}

/// Replace the first EXISTS subquery that holds a long relationship.
fn replace_exists(e: &mut Expr, with: &dyn Fn(&Expr) -> Option<Expr>) -> bool {
    if let Expr::Subquery { patterns, .. } = e {
        if patterns.iter().any(pattern_long) {
            if let Some(x) = with(e) {
                *e = x;
                return true;
            }
            return false;
        }
    }
    let mut go = |x: &mut Expr| replace_exists(x, with);
    match e {
        Expr::Prop(b, _) | Expr::Not(b) | Expr::Neg(b) | Expr::IsNull(b, _) => go(b),
        Expr::Binary(_, a, b) => go(a) || go(b),
        Expr::Func { args, .. } | Expr::List(args) => args.iter_mut().any(go),
        Expr::Map(kv) => kv.iter_mut().any(|(_, v)| go(v)),
        Expr::Case { operand, whens, else_ } => {
            operand.as_deref_mut().is_some_and(&mut go)
                || whens.iter_mut().any(|(a, b)| go(a) || go(b))
                || else_.as_deref_mut().is_some_and(&mut go)
        }
        Expr::ListComp { source, filter, project, .. } => {
            go(source) || filter.as_deref_mut().is_some_and(&mut go) || project.as_deref_mut().is_some_and(&mut go)
        }
        Expr::ListPred { source, filter, .. } => go(source) || go(filter),
        _ => false,
    }
}

fn find_exists(e: &Expr) -> Option<&Expr> {
    if let Expr::Subquery { patterns, .. } = e {
        if patterns.iter().any(pattern_long) {
            return Some(e);
        }
    }
    fn go(x: &Expr) -> Option<&Expr> {
        find_exists(x)
    }
    match e {
        Expr::Prop(b, _) | Expr::Not(b) | Expr::Neg(b) | Expr::IsNull(b, _) => go(b),
        Expr::Binary(_, a, b) => go(a).or_else(|| go(b)),
        Expr::Func { args, .. } | Expr::List(args) => args.iter().find_map(go),
        Expr::Map(kv) => kv.iter().find_map(|(_, v)| go(v)),
        Expr::Case { operand, whens, else_ } => operand
            .as_deref()
            .and_then(go)
            .or_else(|| whens.iter().find_map(|(a, b)| go(a).or_else(|| go(b))))
            .or_else(|| else_.as_deref().and_then(go)),
        Expr::ListComp { source, filter, project, .. } => {
            go(source).or_else(|| filter.as_deref().and_then(go)).or_else(|| project.as_deref().and_then(go))
        }
        Expr::ListPred { source, filter, .. } => go(source).or_else(|| go(filter)),
        _ => None,
    }
}

/// Plan the walk and the statement around it.
pub fn plan(q: &Query, p: &mut Printer, ncols: usize) -> Result<Reach, String> {
    if q.union.as_ref().is_some_and(|(_, t)| query_longs(t) > 0) {
        return Err(unsupported("a long variable-length relationship after UNION"));
    }
    let mut site: Option<(usize, bool)> = None; // (clause, in a pattern of it)
    for (i, c) in q.clauses.iter().enumerate() {
        let (pat, ex) = clause_longs(c);
        if pat + ex == 0 {
            continue;
        }
        if site.is_some() || pat + ex > 1 {
            return Err("more than one long variable-length relationship in a statement is not run on the AGE backend".into());
        }
        site = Some((i, pat == 1));
    }
    let Some((i, in_pattern)) = site else { return Err("no long variable-length relationship".into()) };
    let param = p.fresh("reach");
    let before = scope_of(&q.clauses[..i]);
    let mut main = q.clone();

    let (seed_clauses, start, rel) = if in_pattern {
        // B: the only pattern of a MATCH
        let Clause::Match { patterns, optional, where_ } = &q.clauses[i] else {
            let r = first_long_rel(&q.clauses[i]);
            return Err(refuse(&r, "it is not in a MATCH (CREATE and MERGE make one relationship)"));
        };
        let r = patterns.iter().find_map(|pt| three(pt).map(|(_, r, _)| r).filter(|r| long(r))).cloned()
            .unwrap_or_else(|| first_long_rel(&q.clauses[i]));
        if patterns.len() != 1 {
            return Err(refuse(&r, "the MATCH has more than one pattern"));
        }
        let pat = &patterns[0];
        let Some((a, r, b)) = three(pat) else { return Err(refuse(&r, "the pattern has more than one relationship")) };
        if pat.path_var.is_some() {
            return Err(refuse(r, "a path variable"));
        }
        plain_rel(r).map_err(|w| refuse(r, &w))?;
        let (Some(av), Some(bv)) = (&a.var, &b.var) else {
            return Err(refuse(r, "an end without a variable"));
        };
        if av == bv {
            return Err(refuse(r, "both ends are the same variable"));
        }
        let a_bound = before.contains(av);
        let b_bound = before.contains(bv);
        // start: the end bound before; else the one with properties
        let (s, s_first) = match (a_bound, b_bound) {
            (true, false) => (a, true),
            (false, true) => (b, false),
            (false, false) if !a.props.is_empty() => (a, true),
            (false, false) if !b.props.is_empty() => (b, false),
            (true, true) => return Err(refuse(r, "both ends are bound already (a test, not a match: use EXISTS { })")),
            (false, false) => return Err(refuse(r, "neither end is bound or given properties to start from")),
        };
        let t = if s_first { b } else { a };
        let (sv, tv) = (s.var.clone().unwrap(), t.var.clone().unwrap());
        // what follows must not tell trails from end nodes
        insensitive(&q.clauses[i + 1..], &tv).map_err(|w| refuse(r, &w))?;
        let mut seed: Vec<Clause> = q.clauses[..i].to_vec();
        let s_bound = before.contains(&sv);
        if !s_bound {
            seed.push(Clause::Match {
                patterns: vec![Pattern { path_var: None, elems: vec![PatElem::Node(s.clone())] }],
                optional: false,
                where_: None,
            });
        }
        // the statement: (s), (t) WHERE t is reached from s, AND the MATCH's own WHERE
        let mut pats = Vec::new();
        if !s_bound || !s.labels.is_empty() || !s.props.is_empty() {
            pats.push(Pattern { path_var: None, elems: vec![PatElem::Node(s.clone())] });
        }
        pats.push(Pattern { path_var: None, elems: vec![PatElem::Node(t.clone())] });
        main.clauses[i] = Clause::Match {
            patterns: pats,
            optional: *optional,
            where_: and_all(Some(reached(&param, &sv, &tv)), where_.iter().cloned().collect()),
        };
        let dir = if s_first { r.dir.clone() } else { flip(&r.dir) };
        (seed, sv, RelSeen { types: r.types.clone(), dir, range: r.range.unwrap() })
    } else {
        // A: EXISTS { … } in an expression of the clause
        let c = &q.clauses[i];
        let (where_first, exprs): (bool, Vec<&Expr>) = match c {
            Clause::Match { where_: Some(w), .. } => (true, vec![w]),
            Clause::With { proj, where_ } => {
                let mut v = Vec::new();
                if let Some(w) = where_ {
                    v.push(w);
                }
                let in_where = v.first().is_some_and(|w| find_exists(w).is_some());
                if !in_where {
                    v = proj_exprs(proj);
                }
                (in_where, v)
            }
            Clause::Return(proj) => (false, proj_exprs(proj)),
            _ => {
                let r = first_long_rel(c);
                return Err(refuse(&r, "it is in a subquery of a clause the gateway cannot cut there"));
            }
        };
        let Some(Expr::Subquery { kind, patterns, where_, .. }) = exprs.iter().find_map(|e| find_exists(e)) else {
            let r = first_long_rel(c);
            return Err(refuse(&r, "it is not in EXISTS { }"));
        };
        let r0 = patterns.iter().find_map(|pt| three(pt).map(|(_, r, _)| r).filter(|r| long(r))).cloned()
            .unwrap_or_else(|| first_long_rel(c));
        if *kind != SubqueryKind::Exists {
            return Err(refuse(&r0, "COUNT { } or a pattern comprehension counts trails"));
        }
        if patterns.len() != 1 || where_.is_some() {
            return Err(refuse(&r0, "EXISTS { } with more than the one pattern"));
        }
        let pat = &patterns[0];
        let Some((a, r, b)) = three(pat) else { return Err(refuse(&r0, "the pattern has more than one relationship")) };
        if pat.path_var.is_some() {
            return Err(refuse(r, "a path variable"));
        }
        plain_rel(r).map_err(|w| refuse(r, &w))?;
        if !a.labels.is_empty() || !a.props.is_empty() || !b.labels.is_empty() || !b.props.is_empty() {
            return Err(refuse(r, "labels or properties on its ends inside EXISTS { }"));
        }
        let (Some(av), Some(bv)) = (&a.var, &b.var) else { return Err(refuse(r, "an end without a variable")) };
        // the ends the expression sees, and those known before it (the seeds' query)
        let mut seed: Vec<Clause> = q.clauses[..i].to_vec();
        let visible = match c {
            Clause::With { proj, .. } if where_first => {
                seed.push(Clause::With { proj: Projection { order: vec![], skip: None, limit: None, ..proj.clone() }, where_: None });
                scope_of(&seed)
            }
            Clause::Match { .. } => scope_after(&before, c),
            _ => before.clone(),
        };
        if !visible.contains(av) || !visible.contains(bv) {
            return Err(refuse(r, "an end of it is a new variable (EXISTS { } is walked between two bound nodes)"));
        }
        let known = scope_of(&seed);
        let (sv, tv, s_first) = if known.contains(av) {
            (av.clone(), bv.clone(), true)
        } else if known.contains(bv) {
            (bv.clone(), av.clone(), false)
        } else {
            return Err(refuse(r, "both ends are bound in the same clause (nothing to start the walk from)"));
        };
        let x = reached(&param, &sv, &tv);
        let target = c.clone();
        let mut target = target;
        let replace = |_: &Expr| Some(x.clone());
        let done = match &mut target {
            Clause::Match { where_: Some(w), .. } => replace_exists(w, &replace),
            Clause::With { proj, where_ } => {
                where_.as_mut().is_some_and(|w| replace_exists(w, &replace))
                    || proj.items.iter_mut().any(|it| replace_exists(&mut it.expr, &replace))
                    || proj.order.iter_mut().any(|o| replace_exists(&mut o.expr, &replace))
            }
            Clause::Return(proj) => {
                proj.items.iter_mut().any(|it| replace_exists(&mut it.expr, &replace))
                    || proj.order.iter_mut().any(|o| replace_exists(&mut o.expr, &replace))
            }
            _ => false,
        };
        if !done {
            return Err(refuse(r, "the gateway could not put the test in its place"));
        }
        main.clauses[i] = target;
        let dir = if s_first { r.dir.clone() } else { flip(&r.dir) };
        (seed, sv, RelSeen { types: r.types.clone(), dir, range: r.range.unwrap() })
    };
    if is_write(&Query { clauses: seed_clauses.clone(), union: None }) {
        return Err("a write before a long variable-length relationship is not run on the AGE backend".into());
    }
    let mut seed_q = Query { clauses: seed_clauses, union: None };
    seed_q.clauses.push(seed_return(&start));
    let seed = p.query(&seed_q)?;
    let text = p.query(&main)?;
    Ok(Reach {
        seed,
        types: rel.types,
        dir: rel.dir,
        lo: rel.range.0,
        hi: rel.range.1,
        param,
        text,
        ncols,
    })
}

struct RelSeen {
    types: Vec<String>,
    dir: Dir,
    range: (u32, u32),
}

fn flip(d: &Dir) -> Dir {
    match d {
        Dir::Out => Dir::In,
        Dir::In => Dir::Out,
        Dir::Both => Dir::Both,
    }
}

fn first_long_rel(c: &Clause) -> RelPat {
    fn in_pattern(p: &Pattern) -> Option<RelPat> {
        p.elems.iter().find_map(|e| match e {
            PatElem::Rel(r) if long(r) => Some(r.clone()),
            _ => None,
        })
    }
    fn in_expr(e: &Expr) -> Option<RelPat> {
        if let Expr::Subquery { patterns, .. } = e {
            if let Some(r) = patterns.iter().find_map(in_pattern) {
                return Some(r);
            }
        }
        None
    }
    let found = match c {
        Clause::Match { patterns, where_, .. } => {
            patterns.iter().find_map(in_pattern).or_else(|| where_.as_ref().and_then(|w| find_exists(w)).and_then(in_expr))
        }
        Clause::Create(ps) => ps.iter().find_map(in_pattern),
        Clause::Merge { pattern, .. } => in_pattern(pattern),
        _ => None,
    };
    found.unwrap_or(RelPat { var: None, types: vec![], dir: Dir::Both, props: vec![], range: Some((1, MAX_UNTYPED_HOPS + 1)) })
}

fn has_agg(e: &Expr, plain: &mut bool, any: &mut bool) {
    if let Expr::Func { name, args, distinct } = e {
        let n = name.to_ascii_lowercase();
        if super::cypher::is_agg_call(e) {
            *any = true;
            if !(*distinct || n == "min" || n == "max") {
                *plain = true;
            }
        }
        for a in args {
            has_agg(a, plain, any);
        }
        return;
    }
    let mut go = |x: &Expr| has_agg(x, plain, any);
    match e {
        Expr::Prop(b, _) | Expr::Not(b) | Expr::Neg(b) | Expr::IsNull(b, _) => go(b),
        Expr::Binary(_, a, b) => {
            go(a);
            go(b);
        }
        Expr::List(xs) => xs.iter().for_each(go),
        Expr::Map(kv) => kv.iter().for_each(|(_, v)| go(v)),
        Expr::Case { operand, whens, else_ } => {
            if let Some(o) = operand {
                go(o);
            }
            for (a, b) in whens {
                go(a);
                go(b);
            }
            if let Some(x) = else_ {
                go(x);
            }
        }
        Expr::MapProjection { items, .. } => {
            for i in items {
                if let MapProjItem::Entry(_, v) = i {
                    go(v);
                }
            }
        }
        _ => {}
    }
}

/// Can the clauses after the walk tell one row per trail from one row per
/// reached node? Only reads may come before the first projection, and that
/// projection must collapse duplicates.
fn insensitive(rest: &[Clause], _t: &str) -> Result<(), String> {
    for c in rest {
        match c {
            Clause::Match { .. } | Clause::Unwind { .. } => continue,
            Clause::With { proj, .. } | Clause::Return(proj) => {
                if proj.distinct {
                    return Ok(());
                }
                let (mut plain, mut any) = (false, false);
                for it in &proj.items {
                    has_agg(&it.expr, &mut plain, &mut any);
                }
                if any {
                    return if plain {
                        Err("an aggregate without DISTINCT after it counts one row per trail".into())
                    } else {
                        Ok(())
                    };
                }
                if proj.skip.is_none() && matches!(proj.limit, Some(Expr::Lit(Lit::Int(1)))) {
                    return Ok(());
                }
                return Err("what follows keeps one row per trail (use DISTINCT, a DISTINCT aggregate or LIMIT 1)".into());
            }
            _ => return Err("a write follows it, once per trail".into()),
        }
    }
    Err("nothing collapses the rows after it (one row per trail)".into())
}

// ------------------------------------------------------------------ running

/// The edges incident to some nodes, by the walk's types and direction:
/// `(edge id, from, to)` where `from` is the node the walk is at.
fn fetch(x: &mut dyn Exec, tables: &[String], dir: &Dir, nodes: &[i64]) -> Result<Vec<(i64, i64, i64)>, Refusal> {
    if tables.is_empty() || nodes.is_empty() {
        return Ok(Vec::new());
    }
    let arr = format!(
        "'{{{}}}'::ag_catalog.graphid[]",
        nodes.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(",")
    );
    let cond = match dir {
        Dir::Out => format!("start_id = ANY ({arr})"),
        Dir::In => format!("end_id = ANY ({arr})"),
        Dir::Both => format!("start_id = ANY ({arr}) OR end_id = ANY ({arr})"),
    };
    let sql = tables
        .iter()
        .map(|t| format!("SELECT id::text, start_id::text, end_id::text FROM {t} WHERE {cond}"))
        .collect::<Vec<_>>()
        .join(" UNION ALL ");
    let rows = x.sql(&sql)?;
    let want: HashSet<i64> = nodes.iter().copied().collect();
    let num = |v: &Option<String>| v.as_deref().and_then(|s| s.parse::<i64>().ok()).unwrap_or(-1);
    let mut out = Vec::new();
    for r in rows {
        let (e, a, b) = (num(&r[0]), num(&r[1]), num(&r[2]));
        match dir {
            Dir::Out => out.push((e, a, b)),
            Dir::In => out.push((e, b, a)),
            Dir::Both => {
                if want.contains(&a) {
                    out.push((e, a, b));
                }
                if want.contains(&b) && a != b {
                    out.push((e, b, a));
                }
            }
        }
    }
    Ok(out)
}

/// The label tables of the walk's types (`_ag_label_edge` for any type).
fn tables(x: &mut dyn Exec, graph: &str, types: &[String]) -> Result<Vec<String>, Refusal> {
    let which = if types.is_empty() {
        "l.name = '_ag_label_edge'".to_string()
    } else {
        format!("l.name::text IN ({})", types.iter().map(|t| sql_lit(t)).collect::<Vec<_>>().join(", "))
    };
    let rows = x.sql(&format!(
        "SELECT l.relation::regclass::text FROM ag_catalog.ag_label l JOIN ag_catalog.ag_graph g ON g.graphid = l.graph \
         WHERE g.name = {}::name AND l.kind = 'e' AND {which} ORDER BY 1",
        sql_lit(graph)
    ))?;
    Ok(rows.into_iter().filter_map(|r| r.into_iter().next().flatten()).collect())
}

/// One seed's breadth-first state.
struct Walk {
    start: i64,
    depth: HashMap<i64, u32>,
    /// The relationship each node was reached by (none for the start).
    tree: HashMap<i64, i64>,
    /// The first relationship on the tree path from the start.
    branch: HashMap<i64, i64>,
    frontier: Vec<i64>,
    /// The shortest closed trail through the start seen so far.
    cycle: Option<u32>,
}

const ROOT: i64 = -1;

/// The ids each seed reaches. `adj` is read through `fetch`, a hop at a time,
/// for every seed's frontier at once; each node's edges are read once.
pub fn walk(
    seeds: &[i64],
    dir: &Dir,
    lo: u32,
    hi: u32,
    mut fetch: impl FnMut(&[i64]) -> Result<Vec<(i64, i64, i64)>, Refusal>,
) -> Result<HashMap<i64, Vec<i64>>, Refusal> {
    let mut adj: HashMap<i64, Vec<(i64, i64)>> = HashMap::new();
    let mut walks: Vec<Walk> = seeds
        .iter()
        .map(|&s| Walk {
            start: s,
            depth: HashMap::from([(s, 0)]),
            tree: HashMap::new(),
            branch: HashMap::from([(s, ROOT)]),
            frontier: vec![s],
            cycle: None,
        })
        .collect();
    for d in 0..hi {
        let mut need: Vec<i64> = walks.iter().flat_map(|w| w.frontier.iter().copied()).filter(|n| !adj.contains_key(n)).collect();
        need.sort_unstable();
        need.dedup();
        if !need.is_empty() {
            for n in &need {
                adj.insert(*n, Vec::new());
            }
            for (e, from, to) in fetch(&need)? {
                if let Some(v) = adj.get_mut(&from) {
                    v.push((e, to));
                }
            }
        }
        let mut any = false;
        for w in &mut walks {
            let mut next = Vec::new();
            for &u in &w.frontier {
                for &(e, v) in adj.get(&u).map(Vec::as_slice).unwrap_or(&[]) {
                    if !w.depth.contains_key(&v) {
                        w.depth.insert(v, d + 1);
                        w.tree.insert(v, e);
                        w.branch.insert(v, if u == w.start { e } else { w.branch[&u] });
                        next.push(v);
                        continue;
                    }
                    // v seen before: a way back to the start?
                    let len = match dir {
                        Dir::Out | Dir::In => (v == w.start).then_some(d + 1),
                        Dir::Both => {
                            if u == v {
                                (u == w.start).then_some(1)
                            } else if w.tree.get(&v) == Some(&e) || w.tree.get(&u) == Some(&e) {
                                None
                            } else if w.branch[&u] != w.branch[&v] {
                                Some(w.depth[&u] + w.depth[&v] + 1)
                            } else {
                                None
                            }
                        }
                    };
                    if let Some(l) = len {
                        w.cycle = Some(w.cycle.map_or(l, |c| c.min(l)));
                    }
                }
            }
            any |= !next.is_empty();
            w.frontier = next;
        }
        if !any {
            break;
        }
    }
    let mut out = HashMap::new();
    for w in walks {
        let mut ids: Vec<i64> = w.depth.iter().filter(|(n, k)| **n != w.start && **k >= 1).map(|(n, _)| *n).collect();
        if lo == 0 || w.cycle.is_some_and(|c| c <= hi) {
            ids.push(w.start);
        }
        ids.sort_unstable();
        out.insert(w.start, ids);
    }
    Ok(out)
}

/// Seeds, walk, statement.
pub fn run(x: &mut dyn Exec, r: &Reach, params: &Json) -> Result<Vec<Vec<Ag>>, Refusal> {
    let mut seeds: Vec<i64> = x
        .cypher(&r.seed, 1, params)?
        .into_iter()
        .filter_map(|row| match row.into_iter().next() {
            Some(Ag::Int(i)) => Some(i),
            _ => None,
        })
        .collect();
    seeds.sort_unstable();
    seeds.dedup();
    let graph = x.graph();
    let tables = if seeds.is_empty() { Vec::new() } else { tables(x, &graph, &r.types)? };
    let reached = walk(&seeds, &r.dir, r.lo, r.hi, |nodes| fetch(x, &tables, &r.dir, nodes))?;
    let mut m = Map::new();
    for (s, ids) in reached {
        m.insert(s.to_string(), Json::Array(ids.into_iter().map(Json::from).collect()));
    }
    let mut p = params.clone();
    if !p.is_object() {
        p = Json::Object(Map::new());
    }
    p.as_object_mut().unwrap().insert(r.param.clone(), Json::Object(m));
    x.cypher(&r.text, r.ncols, &p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_of(q: &str) -> Result<Reach, String> {
        let ast = cypher_syntax::parse(q).unwrap();
        assert!(needed(&ast));
        let mut p = Printer::new(None, "2026-11-18T00:00:00+00:00".into());
        let n = super::super::cypher::columns(&ast).len();
        plan(&ast, &mut p, n)
    }

    /// Edges as (id, start, end); the walk reads them through `fetch`.
    fn graph_walk(edges: &[(i64, i64, i64)], seeds: &[i64], dir: Dir, lo: u32, hi: u32) -> (HashMap<i64, Vec<i64>>, usize) {
        let mut calls = 0;
        let out = walk(seeds, &dir, lo, hi, |nodes| {
            calls += 1;
            let mut v = Vec::new();
            for &(e, a, b) in edges {
                match dir {
                    Dir::Out if nodes.contains(&a) => v.push((e, a, b)),
                    Dir::In if nodes.contains(&b) => v.push((e, b, a)),
                    Dir::Both => {
                        if nodes.contains(&a) {
                            v.push((e, a, b));
                        }
                        if nodes.contains(&b) && a != b {
                            v.push((e, b, a));
                        }
                    }
                    _ => {}
                }
            }
            Ok(v)
        })
        .unwrap();
        (out, calls)
    }

    #[test]
    fn robos_three_walks_are_planned() {
        // storyboard_resolver.py:56 — EXISTS between two bound nodes, started from u (bound first)
        let r = plan_of(
            "MATCH (u:UI {id: $uid}) MATCH (c:Command) WHERE NOT EXISTS { (:Policy)-[:INVOKES]->(c) } \
             AND EXISTS { MATCH (c)-[:EMITS|TRIGGERS|INVOKES*1..30]-(u) } RETURN c.id AS id ORDER BY c.id LIMIT 1",
        )
        .unwrap();
        assert_eq!(r.seed, "MATCH (u:UI {id: $uid}) RETURN DISTINCT id(coalesce(u)) AS __og_seed");
        assert!(r.text.contains("(u IS NOT NULL AND c IS NOT NULL AND id(coalesce(c)) IN coalesce($__og_reach1[toString(id(coalesce(u)))], []))"), "{}", r.text);
        assert!(!r.text.contains("*1..30"), "{}", r.text);
        assert_eq!((r.types.len(), r.dir.clone(), r.lo, r.hi), (3, Dir::Both, 1, 30));
        // storyboard_resolver.py:93 — count(DISTINCT n)
        let r = plan_of(
            "MATCH (c:Command {id: $cid}) OPTIONAL MATCH (c)-[:EMITS|TRIGGERS*1..30]-(n) RETURN count(DISTINCT n) AS stepCount",
        )
        .unwrap();
        assert_eq!(r.seed, "MATCH (c:Command {id: $cid}) RETURN DISTINCT id(coalesce(c)) AS __og_seed");
        assert!(r.text.starts_with("MATCH (c:Command {id: $cid}) OPTIONAL MATCH (n) WHERE (c IS NOT NULL AND n IS NOT NULL"), "{}", r.text);
        // repository.py:680 — the start is new, has properties; walked against the arrow from u
        let r = plan_of(
            "UNWIND $ids AS uid OPTIONAL MATCH (u:UI {id: uid})<-[*1..30]-(c:Command) \
             WHERE c IS NOT NULL AND NOT EXISTS { (:Policy)-[:INVOKES]->(c) } WITH uid, c LIMIT 1 \
             OPTIONAL MATCH (m:StoryboardPageMapping {commandId: c.id}) RETURN uid AS id, coalesce(m.status, 'active') = 'archived' AS archived",
        )
        .unwrap();
        assert_eq!(r.seed, "UNWIND $ids AS uid MATCH (u:UI {id: uid}) RETURN DISTINCT id(coalesce(u)) AS __og_seed");
        assert!(r.text.contains("OPTIONAL MATCH (u:UI {id: uid}), (c:Command) WHERE (u IS NOT NULL"), "{}", r.text);
        assert_eq!((r.types.len(), r.dir.clone()), (0, Dir::In));
    }

    #[test]
    fn trails_are_refused_where_they_count() {
        for (q, why) in [
            ("MATCH (a {id: 1}) MATCH (a)-[*1..30]-(b) RETURN b", "one row per trail"),
            ("MATCH (a {id: 1}) MATCH (a)-[*1..30]-(b) RETURN count(*) AS n", "without DISTINCT"),
            ("MATCH (a {id: 1}) MATCH (a)-[r*1..30]-(b) RETURN DISTINCT b", "a variable on it"),
            ("MATCH (a {id: 1}) MATCH p = (a)-[*1..30]-(b) RETURN DISTINCT b", "a path variable"),
            ("MATCH (a {id: 1}) MATCH (a)-[*2..30]-(b) RETURN DISTINCT b", "lower bound"),
            ("MATCH (a {id: 1}) MATCH (a)-[*1..30]-(b) WITH b LIMIT 2 RETURN b", "one row per trail"),
            ("MATCH (a {id: 1}) MATCH (a)-[*1..30]-(b) SET b.x = 1 RETURN DISTINCT b", "a write follows"),
            ("MATCH (a), (b) WHERE COUNT { (a)-[*1..30]-(b) } > 0 RETURN a", "counts trails"),
            ("MATCH (a {id: 1}), (b) WHERE EXISTS { (a)-[*1..30]-(b) } RETURN a", "same clause"),
        ] {
            let e = plan_of(q).err().unwrap_or_else(|| panic!("planned: {q}"));
            assert!(e.contains(why), "{q}: {e}");
        }
        // two in one statement
        assert!(plan_of("MATCH (a {id: 1}) OPTIONAL MATCH (a)-[*1..30]-(b) WITH DISTINCT b MATCH (b)-[*1..30]-(c) RETURN DISTINCT c")
            .err()
            .unwrap()
            .contains("more than one"));
        // one type is AGE's own; not this module's
        assert!(!needed(&cypher_syntax::parse("MATCH (a)-[:X*1..30]-(b) RETURN b").unwrap()));
    }

    #[test]
    fn the_walk_is_neo4js_trails() {
        // 1 - 2 - 3 - 4 (a path) and 1 - 5 (a branch): from 1, undirected
        let path = [(10, 1, 2), (11, 2, 3), (12, 3, 4), (13, 1, 5)];
        let (r, _) = graph_walk(&path, &[1], Dir::Both, 1, 30);
        assert_eq!(r[&1], vec![2, 3, 4, 5]); // no cycle: the start is not reached
        let (r, _) = graph_walk(&path, &[1], Dir::Both, 1, 2);
        assert_eq!(r[&1], vec![2, 3, 5]);
        let (r, _) = graph_walk(&path, &[1], Dir::Both, 0, 2);
        assert_eq!(r[&1], vec![1, 2, 3, 5]);
        // going back over the same relationship is no trail; a second one between the same nodes is
        let (r, _) = graph_walk(&[(10, 1, 2), (11, 2, 1)], &[1], Dir::Both, 1, 30);
        assert_eq!(r[&1], vec![1, 2]);
        // a triangle 1-2-3: the start on a cycle of 3
        let tri = [(10, 1, 2), (11, 2, 3), (12, 3, 1)];
        assert_eq!(graph_walk(&tri, &[1], Dir::Both, 1, 3).0[&1], vec![1, 2, 3]);
        assert_eq!(graph_walk(&tri, &[1], Dir::Both, 1, 2).0[&1], vec![2, 3]);
        // a cycle that does not pass the start: 1 - 2, 2-3-4-2
        let side = [(10, 1, 2), (11, 2, 3), (12, 3, 4), (13, 4, 2)];
        assert_eq!(graph_walk(&side, &[1], Dir::Both, 1, 30).0[&1], vec![2, 3, 4]);
        // directed: only along the arrows, and back to the start on a directed cycle
        assert_eq!(graph_walk(&path, &[1], Dir::Out, 1, 30).0[&1], vec![2, 3, 4, 5]);
        assert_eq!(graph_walk(&path, &[4], Dir::Out, 1, 30).0[&4], Vec::<i64>::new());
        assert_eq!(graph_walk(&path, &[4], Dir::In, 1, 30).0[&4], vec![1, 2, 3]);
        assert_eq!(graph_walk(&tri, &[1], Dir::Out, 1, 3).0[&1], vec![1, 2, 3]);
        assert_eq!(graph_walk(&tri, &[1], Dir::Out, 1, 2).0[&1], vec![2, 3]);
        // a self-loop on the start is a trail of one
        assert_eq!(graph_walk(&[(10, 1, 1)], &[1], Dir::Both, 1, 30).0[&1], vec![1]);
        // two seeds share the reads: one fetch per hop at most
        let (r, calls) = graph_walk(&path, &[1, 3], Dir::Both, 1, 30);
        assert_eq!((r[&1].clone(), r[&3].clone()), (vec![2, 3, 4, 5], vec![1, 2, 4, 5]));
        assert!(calls <= 4, "{calls}");
    }

    #[test]
    fn the_walk_matches_brute_force_trails() {
        // every trail of every small random multigraph, against the walk
        let mut seed = 7u64;
        let mut rnd = |n: u64| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        for case in 0..300 {
            let nodes = 2 + rnd(6) as i64;
            let m = rnd(9) as i64;
            let edges: Vec<(i64, i64, i64)> = (0..m).map(|e| (100 + e, rnd(nodes as u64) as i64, rnd(nodes as u64) as i64)).collect();
            let hi = 1 + rnd(5) as u32;
            for dir in [Dir::Both, Dir::Out] {
                let (got, _) = graph_walk(&edges, &[0], dir.clone(), 1, hi);
                // brute force: depth-first over trails
                let mut seen: HashSet<i64> = HashSet::new();
                fn dfs(at: i64, used: &mut Vec<i64>, hi: u32, edges: &[(i64, i64, i64)], dir: &Dir, seen: &mut HashSet<i64>) {
                    if used.len() as u32 == hi {
                        return;
                    }
                    for &(e, a, b) in edges {
                        if used.contains(&e) {
                            continue;
                        }
                        let next = match dir {
                            Dir::Out => (a == at).then_some(b),
                            _ => if a == at { Some(b) } else if b == at { Some(a) } else { None },
                        };
                        if let Some(n) = next {
                            used.push(e);
                            seen.insert(n);
                            dfs(n, used, hi, edges, dir, seen);
                            used.pop();
                        }
                    }
                }
                dfs(0, &mut Vec::new(), hi, &edges, &dir, &mut seen);
                let mut want: Vec<i64> = seen.into_iter().collect();
                want.sort_unstable();
                assert_eq!(got[&0], want, "case {case} {dir:?} hi {hi} edges {edges:?}");
            }
        }
    }
}
