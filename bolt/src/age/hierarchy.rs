//! Labels that come with other labels: PostgreSQL inheritance between AGE's
//! label tables (STUDIO-ON-AGE G1, G2, G3, G6; ANALYZER-ON-AGE §5 item 1).
//!
//! A vertex in AGE is a row of one label table, so it has one label. Neo4j's
//! clients give a node several — an umbrella and a kind, `(:_Entity:Customer)`,
//! `(:ANALYZER_OWNED:TABLE)` — and find it by either. Here a label table may
//! inherit others (`ALTER TABLE <graph>."Customer" INHERIT <graph>."_Entity"`),
//! and a node **has label L when the table it is stored in is L's, or inherits
//! L's** (transitively): its labels are its table's and that table's
//! ancestors'.
//!
//! - **Storage** ([`Hierarchy::store`]): `CREATE`/`MERGE (n:A:B:C {…})` stores
//!   the node under the last label, C — or under one of the labels given whose
//!   table inherits all the others already — and before the statement, in its
//!   transaction, the missing label tables are made with AGE's
//!   `create_vlabel` and C's table is made to inherit A's and B's ([`Schema`],
//!   [`apply`]). Refused: C holding nodes already that would gain A silently,
//!   a cycle, a relationship type among the labels.
//! - **Reads**: `MATCH (n:L)` scans L, and AGE's scan takes the inheriting
//!   tables along (its label scans are made without `ONLY`, so PostgreSQL's
//!   inheritance expansion applies) — `(n:_Entity)` finds the Customers.
//!   `(n:A:B)`, `(n:A|B)`, `n:L` and `labels(n)` are printed with the hierarchy
//!   (`cypher.rs` `node_label_test`, `HasLabels`, `labels()`).
//! - **Label changes** ([`Hierarchy::relabel`], run by `split.rs`): a vertex
//!   moved to another table takes the old table's parents to the new one;
//!   `SET n:L` of a label it has through its table is nothing to do; `REMOVE
//!   n:L` of such a label is refused.
//!
//! The hierarchy is read from the catalog ([`sql`]: `ag_catalog.ag_label`
//! joined with `pg_inherits`) once for each statement that needs it
//! ([`needs`]), and not kept from one statement to the next.

use super::cypher::{distinct, ident, label_groups};
use super::split::Exec;
use super::{sql_ident, sql_lit, Refusal};
use cypher_syntax::ast::*;
use serde_json::{Map, Value as Json};
use std::collections::{BTreeMap, BTreeSet};

/// A graph's labels and which vertex label tables inherit which.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hierarchy {
    /// Read from the catalog (not a statement's that did not need it).
    pub read: bool,
    /// The vertex labels (tables) the graph has.
    pub vertex: BTreeSet<String>,
    /// Its relationship types.
    pub edge: BTreeSet<String>,
    /// A vertex label → the vertex labels its table inherits directly, in
    /// `pg_inherits` order. (`_ag_label_vertex`, which every one inherits, is
    /// left out.)
    parents: BTreeMap<String, Vec<String>>,
}

fn semantic(message: String) -> Refusal {
    Refusal { code: "Neo.ClientError.Statement.SemanticError".into(), message }
}

/// The catalog read: every label of the graph, its kind, and each vertex
/// label its table inherits directly (`NULL` for none: the row of
/// `_ag_label_vertex` / `_ag_label_edge`, which all tables inherit).
pub fn sql(graph: &str) -> String {
    format!(
        "SELECT l.name::text, l.kind::text, p.name::text \
         FROM ag_catalog.ag_label l \
         JOIN ag_catalog.ag_graph g ON g.graphid = l.graph \
         LEFT JOIN pg_catalog.pg_inherits i ON i.inhrelid = l.relation::oid \
         LEFT JOIN ag_catalog.ag_label p ON p.relation::oid = i.inhparent AND p.graph = l.graph \
           AND p.kind = 'v' AND p.name <> '_ag_label_vertex' \
         WHERE g.name = {} AND l.name NOT IN ('_ag_label_vertex', '_ag_label_edge') \
         ORDER BY 1, i.inhseqno",
        sql_lit(graph)
    )
}

/// Read the graph's hierarchy ([`sql`]).
pub fn read(x: &mut dyn Exec) -> Result<Hierarchy, Refusal> {
    let rows = x.sql(&sql(&x.graph()))?;
    Ok(Hierarchy::from_rows(&rows))
}

impl Hierarchy {
    /// From [`sql`]'s rows.
    pub fn from_rows(rows: &[Vec<Option<String>>]) -> Hierarchy {
        let mut h = Hierarchy { read: true, ..Hierarchy::default() };
        for r in rows {
            let (Some(Some(name)), Some(Some(kind))) = (r.first(), r.get(1)) else { continue };
            if kind == "e" {
                h.edge.insert(name.clone());
                continue;
            }
            h.vertex.insert(name.clone());
            if let Some(Some(p)) = r.get(2) {
                let ps = h.parents.entry(name.clone()).or_default();
                if !ps.contains(p) {
                    ps.push(p.clone());
                }
            }
        }
        h
    }

    /// A hierarchy as read, from (child, parent) pairs (both vertex labels) and
    /// labels without parents.
    #[cfg(test)]
    pub fn of(pairs: &[(&str, &str)], others: &[&str]) -> Hierarchy {
        let mut h = Hierarchy { read: true, ..Hierarchy::default() };
        for l in others {
            h.vertex.insert(l.to_string());
        }
        for (c, p) in pairs {
            h.vertex.insert(c.to_string());
            h.vertex.insert(p.to_string());
            h.inherit(c, p);
        }
        h
    }

    /// The labels `l`'s table inherits directly.
    pub fn parents(&self, l: &str) -> &[String] {
        self.parents.get(l).map(Vec::as_slice).unwrap_or(&[])
    }

    /// `l` and every label its table inherits, nearest first: the labels a
    /// node stored under `l` has.
    pub fn up(&self, l: &str) -> Vec<String> {
        let mut out = vec![l.to_string()];
        let mut i = 0;
        while i < out.len() {
            let cur = out[i].clone();
            for p in self.parents(&cur) {
                if !out.contains(p) {
                    out.push(p.clone());
                }
            }
            i += 1;
        }
        out
    }

    /// `l` and every label whose table inherits it (those sorted): the labels
    /// a node that has `l` may be stored under.
    pub fn down(&self, l: &str) -> Vec<String> {
        let mut out = vec![l.to_string()];
        let below: BTreeSet<&String> =
            self.parents.keys().filter(|c| c.as_str() != l && self.up(c).iter().any(|a| a == l)).collect();
        out.extend(below.into_iter().cloned());
        out
    }

    /// No table inherits another: every node has exactly its one label.
    pub fn is_flat(&self) -> bool {
        self.parents.values().all(Vec::is_empty)
    }

    /// The labels whose tables inherit others, each with what `labels()`
    /// gives for a node stored under it ([`Hierarchy::up`]).
    pub fn chains(&self) -> Vec<(String, Vec<String>)> {
        self.parents.iter().filter(|(_, ps)| !ps.is_empty()).map(|(l, _)| (l.clone(), self.up(l))).collect()
    }

    fn inherit(&mut self, child: &str, parent: &str) {
        let ps = self.parents.entry(child.to_string()).or_default();
        if !ps.iter().any(|p| p == parent) {
            ps.push(parent.to_string());
        }
    }

    /// Where a node written with all of `labels` (`CREATE`/`MERGE (n:A:B:C)`)
    /// is stored, and the schema that needs, added to `schema` and to this
    /// hierarchy (which then reads as it will be once `schema` has run).
    ///
    /// - one of the labels whose table inherits all the others already (the
    ///   last such): stored there, nothing to change;
    /// - otherwise the last label, C: the labels without a table get one, and
    ///   C's table inherits every earlier label it does not have yet (the
    ///   nearest first, so `(:A:B:C)` with B inheriting A already needs only
    ///   `C INHERIT B`). A C that existed before must hold no node yet
    ///   ([`Schema::empty`], checked when it runs).
    pub fn store(&mut self, labels: &[String], schema: &mut Schema) -> Result<String, Refusal> {
        let labels = distinct(labels);
        let shape: String = labels.iter().map(|l| format!(":{l}")).collect();
        let Some(last) = labels.last().cloned() else { return Err(semantic("a node pattern without a label".into())) };
        if labels.len() == 1 {
            return Ok(last);
        }
        if let Some(e) = labels.iter().find(|l| self.edge.contains(*l)) {
            return Err(semantic(format!("({shape}) cannot be stored: '{e}' is a relationship type in this graph, not a label")));
        }
        if let Some(t) = labels.iter().rev().find(|t| {
            let up = self.up(t);
            labels.iter().all(|l| up.contains(l))
        }) {
            return Ok(t.clone());
        }
        let existed = self.vertex.contains(&last) && !schema.create.contains(&last);
        let mut added = Vec::new();
        // what `last` will have: its own, and what each label added brings
        let mut cover = self.up(&last);
        for l in labels[..labels.len() - 1].iter().rev() {
            if cover.contains(l) {
                continue;
            }
            if self.up(l).contains(&last) {
                return Err(semantic(format!(
                    "({shape}) cannot be stored on the AGE backend: a node is stored under its last label, {last}, whose \
                     table would have to inherit {l}'s — but {l}'s table inherits {last}'s already. Give the labels \
                     the other way round, ({})",
                    labels.iter().rev().map(|l| format!(":{l}")).collect::<String>()
                )));
            }
            cover.extend(self.up(l));
            added.push(l.clone());
        }
        for l in &labels {
            if !self.vertex.contains(l) {
                self.vertex.insert(l.clone());
                schema.create.push(l.clone());
            }
        }
        for l in &added {
            self.inherit(&last, l);
            schema.inherit.push((last.clone(), l.clone()));
        }
        if existed && !added.is_empty() {
            schema.require_empty(&last, &added, &format!("({shape})"));
        }
        Ok(last)
    }

    /// A vertex stored under `stored` (None: no label) gets `add` and loses
    /// `remove` (`SET n:L` / `REMOVE n:L`): stay, or move to which table.
    /// Schema it needs is added to `schema` and to this hierarchy.
    ///
    /// - a label it has (its own or one its table inherits) set again, or one
    ///   it does not have removed: nothing to do, and not counted;
    /// - a label it has through its table removed, while the table's own label
    ///   stays: refused — it comes with the table;
    /// - otherwise it moves to a table that has every label it is to keep and
    ///   get and none it is to lose: one of those labels, the new ones (last
    ///   first) before the kept ones. When its own label goes and exactly one
    ///   comes (`REMOVE n:Customer SET n:Client`), the new label's table is
    ///   made to inherit the old one's parents first (`Client INHERIT
    ///   _Entity`), as for `CREATE`; a table that existed must hold no node
    ///   then ([`Schema::empty`]);
    /// - no such table: refused, a vertex has one.
    pub fn relabel(
        &mut self,
        var: &str,
        stored: Option<&str>,
        add: &[String],
        remove: &[String],
        schema: &mut Schema,
    ) -> Result<Move, Refusal> {
        let have: Vec<String> = stored.map(|s| self.up(s)).unwrap_or_default();
        let new: Vec<String> = distinct(add).into_iter().filter(|a| !have.contains(a)).collect();
        let gone: Vec<String> = distinct(remove).into_iter().filter(|r| have.contains(r)).collect();
        if new.is_empty() && gone.is_empty() {
            return Ok(Move::Stay);
        }
        let own = stored.unwrap_or_default();
        let drops_own = gone.iter().any(|g| g == own);
        if !gone.is_empty() && !drops_own {
            let g = gone.join(":");
            return Err(semantic(format!(
                "REMOVE {var}:{g} is refused on the AGE backend: `{var}` is stored as {own}, and {own}'s table inherits \
                 {g} — every {own} node is {g}, and one cannot be taken out. Remove {own} as well (the node moves to the \
                 label it keeps), or set a label that does not come with {g}"
            )));
        }
        let kept: Vec<String> = have.iter().filter(|l| !gone.contains(l)).cloned().collect();
        let mut want: Vec<String> = new.clone();
        want.extend(kept.iter().cloned());
        if want.is_empty() {
            return Ok(self.moved(&have, None));
        }
        let fits = |h: &Hierarchy, t: &str| {
            let up = h.up(t);
            want.iter().all(|w| up.contains(w)) && !gone.iter().any(|g| up.contains(g))
        };
        let order: Vec<String> = new.iter().rev().chain(kept.iter()).cloned().collect();
        if let Some(t) = order.iter().find(|t| fits(self, t)) {
            return Ok(self.moved(&have, Some(t)));
        }
        if drops_own && new.len() == 1 {
            let t = new[0].clone();
            if self.edge.contains(&t) {
                return Err(semantic(format!("SET {var}:{t}: '{t}' is a relationship type in this graph, not a label")));
            }
            let existed = self.vertex.contains(&t) && !schema.create.contains(&t);
            let mut added = Vec::new();
            for p in self.parents(own).to_vec() {
                if gone.contains(&p) || self.up(&t).contains(&p) || self.up(&p).contains(&t) {
                    continue;
                }
                if !self.vertex.contains(&t) {
                    self.vertex.insert(t.clone());
                    schema.create.push(t.clone());
                }
                self.inherit(&t, &p);
                schema.inherit.push((t.clone(), p.clone()));
                added.push(p);
            }
            if existed && !added.is_empty() {
                schema.require_empty(&t, &added, &format!("SET {var}:{t}"));
            }
            if fits(self, &t) {
                return Ok(self.moved(&have, Some(&t)));
            }
        }
        Err(semantic(format!(
            "a vertex in AGE has one label (and the labels its table inherits): `{var}` would have {} — set the new one \
             and remove the old one in the same statement",
            want.join(", ")
        )))
    }

    fn moved(&self, had: &[String], to: Option<&str>) -> Move {
        let now = to.map(|t| self.up(t)).unwrap_or_default();
        Move::To {
            label: to.map(str::to_string),
            added: now.iter().filter(|l| !had.contains(l)).count() as i64,
            removed: had.iter().filter(|l| !now.contains(l)).count() as i64,
        }
    }
}

/// What a label change does to one vertex.
#[derive(Debug, PartialEq)]
pub enum Move {
    Stay,
    /// To this label's table (None: `_ag_label_vertex`, no label); the labels
    /// it gains and loses, its table's and their ancestors'.
    To { label: Option<String>, added: i64, removed: i64 },
}

/// The schema a statement needs before it runs: label tables to make and
/// inheritance to add (DIRECTION.md §4.2 rule 5 — schema DDL on the label
/// tables, no data written).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Schema {
    /// `create_vlabel`, in order.
    pub create: Vec<String>,
    /// (child, parent): `ALTER TABLE child INHERIT parent`, in order.
    pub inherit: Vec<(String, String)>,
    /// Tables that existed and get parents: each must hold no node yet (its
    /// nodes, and those of the tables inheriting it, would get the parents'
    /// labels without being asked).
    pub empty: Vec<MustBeEmpty>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MustBeEmpty {
    pub label: String,
    /// The labels its table is to inherit.
    pub parents: Vec<String>,
    /// What asked for it, for the message: `(:A:C)`, `SET n:C`.
    pub what: String,
}

impl Schema {
    pub fn is_empty(&self) -> bool {
        self.create.is_empty() && self.inherit.is_empty() && self.empty.is_empty()
    }

    fn require_empty(&mut self, label: &str, parents: &[String], what: &str) {
        match self.empty.iter_mut().find(|e| e.label == label) {
            Some(e) => {
                for p in parents {
                    if !e.parents.contains(p) {
                        e.parents.push(p.clone());
                    }
                }
            }
            None => self.empty.push(MustBeEmpty { label: label.into(), parents: parents.to_vec(), what: what.into() }),
        }
    }
}

/// `create_vlabel`, unless the label is there (made by another session since
/// the hierarchy was read). AGE 1.8.0 takes `(cstring, cstring)`.
pub fn create_vlabel_sql(graph: &str, label: &str) -> String {
    format!(
        "SELECT ag_catalog.create_vlabel({g}::cstring, {l}::cstring) WHERE NOT EXISTS (SELECT 1 FROM ag_catalog.ag_label x \
         JOIN ag_catalog.ag_graph y ON y.graphid = x.graph WHERE y.name = {g} AND x.name = {l})",
        g = sql_lit(graph),
        l = sql_lit(label)
    )
}

pub fn inherit_sql(graph: &str, child: &str, parent: &str) -> String {
    let g = sql_ident(graph);
    format!("ALTER TABLE {g}.{} INHERIT {g}.{}", sql_ident(child), sql_ident(parent))
}

/// The Cypher that tells whether a label's table (or one inheriting it) holds
/// a node: AGE's own scan, as `MATCH (n:L)` reads it.
pub fn holds_sql(label: &str) -> String {
    format!("MATCH (n:{}) RETURN id(n) LIMIT 1", ident(label))
}

/// Run the schema, in the statement's transaction: the checks first (so a
/// refusal leaves nothing made), then the tables, then the inheritance.
pub fn apply(x: &mut dyn Exec, s: &Schema) -> Result<(), Refusal> {
    let graph = x.graph();
    for e in &s.empty {
        if !x.cypher(&holds_sql(&e.label), 1, &Json::Object(Map::new()))?.is_empty() {
            let ps = e.parents.join(", ");
            return Err(semantic(format!(
                "{} is refused on the AGE backend: a node is stored under one label and has the labels that label's \
                 table inherits, so {l}'s table would have to inherit {ps} — but {l} already has nodes, and they would \
                 all become {ps} as well. Use a label that has no nodes yet, or delete the {l} nodes first",
                e.what,
                l = e.label
            )));
        }
    }
    for l in &s.create {
        x.sql(&create_vlabel_sql(&graph, l))?;
    }
    for (c, p) in &s.inherit {
        x.sql(&inherit_sql(&graph, c, p))?;
    }
    Ok(())
}

// ------------------------------------------------------------ storage

/// The statement with every node of a `CREATE` / `MERGE` written with more
/// than one label (all of them, `(n:A:B)`; a choice `(n:A|B)` is refused when
/// printed) given the one it is stored under ([`Hierarchy::store`]), and the
/// schema that takes. `h` reads as it will after the schema has run. Into
/// FOREACH and CALL { … } bodies and UNION's branches.
pub fn store_all(q: &Query, h: &mut Hierarchy) -> Result<(Query, Schema), Refusal> {
    let mut s = Schema::default();
    let q = store_query(q, h, &mut s)?;
    Ok((q, s))
}

fn store_query(q: &Query, h: &mut Hierarchy, s: &mut Schema) -> Result<Query, Refusal> {
    let clauses = store_clauses(&q.clauses, h, s)?;
    let union = match &q.union {
        Some((all, t)) => Some((*all, Box::new(store_query(t, h, s)?))),
        None => None,
    };
    Ok(Query { clauses, union })
}

fn store_clauses(cs: &[Clause], h: &mut Hierarchy, s: &mut Schema) -> Result<Vec<Clause>, Refusal> {
    let mut out = Vec::with_capacity(cs.len());
    for c in cs {
        out.push(match c {
            Clause::Create(ps) => Clause::Create(ps.iter().map(|p| store_pattern(p, h, s)).collect::<Result<_, _>>()?),
            Clause::Merge { pattern, on_create, on_match } => {
                Clause::Merge { pattern: store_pattern(pattern, h, s)?, on_create: on_create.clone(), on_match: on_match.clone() }
            }
            Clause::Foreach { var, list, body } => {
                Clause::Foreach { var: var.clone(), list: list.clone(), body: store_clauses(body, h, s)? }
            }
            Clause::Subquery { scope, body, optional } => {
                Clause::Subquery { scope: scope.clone(), body: Box::new(store_query(body, h, s)?), optional: *optional }
            }
            other => other.clone(),
        });
    }
    Ok(out)
}

fn store_pattern(p: &Pattern, h: &mut Hierarchy, s: &mut Schema) -> Result<Pattern, Refusal> {
    let mut p = p.clone();
    for el in &mut p.elems {
        let PatElem::Node(n) = el else { continue };
        let groups = label_groups(n);
        if !groups.iter().all(|g| g.len() == 1) {
            continue;
        }
        let labels = distinct(&groups.iter().map(|g| g[0].clone()).collect::<Vec<_>>());
        if labels.len() > 1 {
            let t = h.store(&labels, s)?;
            n.labels = vec![t.clone()];
            n.label_groups = vec![vec![t]];
        }
    }
    Ok(p)
}

// ------------------------------------------------------ what needs it

/// Does the statement need the hierarchy? A node pattern with more than one
/// label (`(n:A:B)`, `(n:A|B)`, in any clause or subquery), a label predicate
/// (`n:L`), `labels()`, a label `SET` / `REMOVE`. A plain `MATCH (n:L)` does
/// not: AGE's scan of L takes the inheriting tables along by itself.
pub fn needs(q: &Query) -> bool {
    q.clauses.iter().any(clause_needs) || q.union.as_ref().is_some_and(|(_, t)| needs(t))
}

fn node_needs(n: &NodePat) -> bool {
    let all: Vec<String> = label_groups(n).into_iter().flatten().collect();
    distinct(&all).len() > 1 || n.props.iter().any(|(_, v)| expr_needs(v))
}

fn pattern_needs(p: &Pattern) -> bool {
    p.elems.iter().any(|el| match el {
        PatElem::Node(n) => node_needs(n),
        PatElem::Rel(r) => r.props.iter().any(|(_, v)| expr_needs(v)),
    })
}

fn ops_need(items: &[SetOp]) -> bool {
    items.iter().any(|i| match i {
        SetOp::Label { .. } => true,
        SetOp::Prop { value, .. } | SetOp::Map { value, .. } => expr_needs(value),
    })
}

fn proj_needs(p: &Projection) -> bool {
    p.items.iter().any(|i| expr_needs(&i.expr))
        || p.order.iter().any(|o| expr_needs(&o.expr))
        || p.skip.as_ref().is_some_and(expr_needs)
        || p.limit.as_ref().is_some_and(expr_needs)
}

fn clause_needs(c: &Clause) -> bool {
    match c {
        Clause::Match { patterns, where_, .. } => patterns.iter().any(pattern_needs) || where_.as_ref().is_some_and(expr_needs),
        Clause::Unwind { expr, .. } => expr_needs(expr),
        Clause::With { proj, where_ } => proj_needs(proj) || where_.as_ref().is_some_and(expr_needs),
        Clause::Return(p) => proj_needs(p),
        Clause::Create(ps) => ps.iter().any(pattern_needs),
        Clause::Merge { pattern, on_create, on_match } => pattern_needs(pattern) || ops_need(on_create) || ops_need(on_match),
        Clause::Set(items) | Clause::Remove(items) => ops_need(items),
        Clause::Delete { exprs, .. } => exprs.iter().any(expr_needs),
        Clause::Call { args, .. } => args.iter().any(expr_needs),
        Clause::Foreach { list, body, .. } => expr_needs(list) || body.iter().any(clause_needs),
        Clause::Subquery { body, .. } => needs(body),
        Clause::Ddl(_) => false,
    }
}

fn expr_needs(e: &Expr) -> bool {
    let opt = |x: &Option<Box<Expr>>| x.as_deref().is_some_and(expr_needs);
    match e {
        Expr::HasLabels { .. } => true,
        Expr::Func { name, args, .. } => name.eq_ignore_ascii_case("labels") || args.iter().any(expr_needs),
        Expr::Subquery { patterns, where_, project, .. } => patterns.iter().any(pattern_needs) || opt(where_) || opt(project),
        Expr::Prop(b, _) | Expr::Not(b) | Expr::Neg(b) | Expr::IsNull(b, _) => expr_needs(b),
        Expr::Binary(_, a, b) => expr_needs(a) || expr_needs(b),
        Expr::List(xs) => xs.iter().any(expr_needs),
        Expr::Map(kv) => kv.iter().any(|(_, v)| expr_needs(v)),
        Expr::Case { operand, whens, else_ } => opt(operand) || whens.iter().any(|(a, b)| expr_needs(a) || expr_needs(b)) || opt(else_),
        Expr::ListComp { source, filter, project, .. } => expr_needs(source) || opt(filter) || opt(project),
        Expr::ListPred { source, filter, .. } => expr_needs(source) || expr_needs(filter),
        Expr::MapProjection { items, .. } => items.iter().any(|i| matches!(i, MapProjItem::Entry(_, v) if expr_needs(v))),
        Expr::Slice { list, from, to } => expr_needs(list) || opt(from) || opt(to),
        Expr::Reduce { init, list, expr, .. } => expr_needs(init) || expr_needs(list) || expr_needs(expr),
        Expr::Lit(_) | Expr::Param(_) | Expr::Var(_) | Expr::Star => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::age::agtype::Ag;

    fn s(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn the_catalog_rows_are_read_into_a_hierarchy() {
        let row = |n: &str, k: &str, p: Option<&str>| vec![Some(n.to_string()), Some(k.to_string()), p.map(str::to_string)];
        let h = Hierarchy::from_rows(&[
            row("Customer", "v", None),
            row("Customer", "v", Some("_Entity")),
            row("PLACED", "e", None),
            row("VIP", "v", None),
            row("VIP", "v", Some("Customer")),
            row("VIP", "v", Some("Rated")),
            row("Rated", "v", None),
            row("_Entity", "v", None),
        ]);
        assert!(h.read && !h.is_flat());
        assert_eq!(h.vertex, ["Customer", "Rated", "VIP", "_Entity"].iter().map(|x| x.to_string()).collect());
        assert!(h.edge.contains("PLACED"));
        // nearest first; a label once
        assert_eq!(h.up("VIP"), s(&["VIP", "Customer", "Rated", "_Entity"]));
        assert_eq!(h.up("_Entity"), s(&["_Entity"]));
        assert_eq!(h.down("_Entity"), s(&["_Entity", "Customer", "VIP"]));
        assert_eq!(h.down("Missing"), s(&["Missing"]));
        assert_eq!(h.chains(), vec![("Customer".into(), s(&["Customer", "_Entity"])), ("VIP".into(), s(&["VIP", "Customer", "Rated", "_Entity"]))]);
        assert!(Hierarchy::from_rows(&[row("A", "v", None)]).is_flat());
        // the catalog read: ag_label joined with pg_inherits, the graph's name quoted
        let q = sql("o'k");
        assert!(q.contains("pg_catalog.pg_inherits i ON i.inhrelid = l.relation::oid") && q.contains("g.name = 'o''k'"), "{q}");
    }

    #[test]
    fn a_node_with_labels_is_stored_under_the_last_one() {
        // a fresh graph: both tables made, the last inherits the first
        let mut h = Hierarchy::of(&[], &[]);
        let mut sc = Schema::default();
        assert_eq!(h.store(&s(&["_Entity", "Customer"]), &mut sc).unwrap(), "Customer");
        assert_eq!(sc, Schema { create: s(&["_Entity", "Customer"]), inherit: vec![("Customer".into(), "_Entity".into())], empty: vec![] });
        // the next one: nothing to do
        let mut sc = Schema::default();
        assert_eq!(h.store(&s(&["_Entity", "Customer"]), &mut sc).unwrap(), "Customer");
        assert!(sc.is_empty());
        // a label given that inherits the others already is where it goes, wherever it stands
        assert_eq!(h.store(&s(&["Customer", "_Entity"]), &mut sc).unwrap(), "Customer");
        assert!(sc.is_empty());
        // three: the nearest first, so what B inherits is not added again
        let mut h = Hierarchy::of(&[("B", "A")], &[]);
        let mut sc = Schema::default();
        assert_eq!(h.store(&s(&["A", "B", "C"]), &mut sc).unwrap(), "C");
        assert_eq!((sc.create.clone(), sc.inherit.clone()), (s(&["C"]), vec![("C".into(), "B".into())]));
        assert_eq!(h.up("C"), s(&["C", "B", "A"]));
        // one label: as it is
        assert_eq!(h.store(&s(&["X", "X"]), &mut Schema::default()).unwrap(), "X");
    }

    #[test]
    fn storing_is_refused_where_it_would_change_other_nodes() {
        // TABLE exists (it may hold nodes): checked when the statement runs
        let mut h = Hierarchy::of(&[], &["TABLE"]);
        let mut sc = Schema::default();
        assert_eq!(h.store(&s(&["ANALYZER_OWNED", "TABLE"]), &mut sc).unwrap(), "TABLE");
        assert_eq!(sc.create, s(&["ANALYZER_OWNED"]));
        assert_eq!(sc.empty, vec![MustBeEmpty { label: "TABLE".into(), parents: s(&["ANALYZER_OWNED"]), what: "(:ANALYZER_OWNED:TABLE)".into() }]);
        // a cycle
        let mut h = Hierarchy::of(&[("B", "A")], &[]);
        let e = h.store(&s(&["B", "X", "A"]), &mut Schema::default()).unwrap_err();
        assert_eq!(e.code, "Neo.ClientError.Statement.SemanticError");
        assert!(e.message.contains("B's table inherits A's already"), "{}", e.message);
        // a relationship type
        let mut h = Hierarchy::of(&[], &[]);
        h.edge.insert("R".into());
        assert!(h.store(&s(&["A", "R"]), &mut Schema::default()).unwrap_err().message.contains("relationship type"));
    }

    /// Records what it was asked; `MATCH` answers with `holding` rows.
    struct Fake {
        asked: Vec<String>,
        holding: usize,
    }

    impl Exec for Fake {
        fn cypher(&mut self, text: &str, _ncols: usize, _params: &Json) -> Result<Vec<Vec<Ag>>, Refusal> {
            self.asked.push(text.to_string());
            Ok(vec![vec![Ag::Int(1)]; self.holding])
        }
        fn sql(&mut self, sql: &str) -> Result<Vec<Vec<Option<String>>>, Refusal> {
            self.asked.push(sql.to_string());
            Ok(Vec::new())
        }
        fn graph(&self) -> String {
            "g".into()
        }
    }

    #[test]
    fn the_schema_runs_checks_then_tables_then_inheritance() {
        let mut h = Hierarchy::of(&[], &["TABLE"]);
        let mut sc = Schema::default();
        h.store(&s(&["ANALYZER_OWNED", "TABLE"]), &mut sc).unwrap();
        let mut f = Fake { asked: vec![], holding: 0 };
        apply(&mut f, &sc).unwrap();
        assert_eq!(
            f.asked,
            vec![
                "MATCH (n:TABLE) RETURN id(n) LIMIT 1".to_string(),
                "SELECT ag_catalog.create_vlabel('g'::cstring, 'ANALYZER_OWNED'::cstring) WHERE NOT EXISTS (SELECT 1 FROM \
                 ag_catalog.ag_label x JOIN ag_catalog.ag_graph y ON y.graphid = x.graph WHERE y.name = 'g' AND x.name = \
                 'ANALYZER_OWNED')"
                    .to_string(),
                "ALTER TABLE \"g\".\"TABLE\" INHERIT \"g\".\"ANALYZER_OWNED\"".to_string(),
            ]
        );
        // TABLE holds nodes: refused before anything is made
        let mut f = Fake { asked: vec![], holding: 1 };
        let e = apply(&mut f, &sc).unwrap_err();
        assert_eq!(f.asked.len(), 1, "{:?}", f.asked);
        assert_eq!(e.code, "Neo.ClientError.Statement.SemanticError");
        assert!(e.message.starts_with("(:ANALYZER_OWNED:TABLE) is refused on the AGE backend") && e.message.contains("TABLE already has nodes"), "{}", e.message);
        // nothing to do: nothing sent
        let mut f = Fake { asked: vec![], holding: 0 };
        apply(&mut f, &Schema::default()).unwrap();
        assert!(f.asked.is_empty());
    }

    #[test]
    fn a_label_change_keeps_what_the_table_gives() {
        let mut h = Hierarchy::of(&[("Customer", "_Entity")], &["Party"]);
        let mut sc = Schema::default();
        // a label it has through its table: nothing to do
        assert_eq!(h.relabel("n", Some("Customer"), &s(&["_Entity"]), &[], &mut sc).unwrap(), Move::Stay);
        assert_eq!(h.relabel("n", Some("Customer"), &s(&["Customer"]), &s(&["Party"]), &mut sc).unwrap(), Move::Stay);
        // and it cannot be taken off one node
        let e = h.relabel("n", Some("Customer"), &[], &s(&["_Entity"]), &mut sc).unwrap_err();
        assert!(e.message.starts_with("REMOVE n:_Entity is refused on the AGE backend: `n` is stored as Customer"), "{}", e.message);
        // the class renamed: the new table takes the old one's parents
        assert_eq!(
            h.relabel("n", Some("Customer"), &s(&["Client"]), &s(&["Customer"]), &mut sc).unwrap(),
            Move::To { label: Some("Client".into()), added: 1, removed: 1 }
        );
        assert_eq!(sc, Schema { create: s(&["Client"]), inherit: vec![("Client".into(), "_Entity".into())], empty: vec![] });
        // the next node: nothing more
        let mut sc = Schema::default();
        assert_eq!(h.relabel("n", Some("Customer"), &s(&["Client"]), &s(&["Customer"]), &mut sc).unwrap(), Move::To { label: Some("Client".into()), added: 1, removed: 1 });
        assert!(sc.is_empty());
        // to a table that exists already: it must be empty
        let mut sc = Schema::default();
        h.relabel("n", Some("Customer"), &s(&["Party"]), &s(&["Customer"]), &mut sc).unwrap();
        assert_eq!(sc.empty, vec![MustBeEmpty { label: "Party".into(), parents: s(&["_Entity"]), what: "SET n:Party".into() }]);
        // its own label removed: it keeps the parent's
        let mut h = Hierarchy::of(&[("Customer", "_Entity")], &[]);
        assert_eq!(h.relabel("n", Some("Customer"), &[], &s(&["Customer"]), &mut sc).unwrap(), Move::To { label: Some("_Entity".into()), added: 0, removed: 1 });
        // both removed: no label
        assert_eq!(h.relabel("n", Some("Customer"), &[], &s(&["Customer", "_Entity"]), &mut sc).unwrap(), Move::To { label: None, added: 0, removed: 2 });
        // a label below its own: down into that table
        assert_eq!(h.relabel("n", Some("_Entity"), &s(&["Customer"]), &[], &mut sc).unwrap(), Move::To { label: Some("Customer".into()), added: 1, removed: 0 });
        // a second label beside its own: refused, as before
        let e = h.relabel("n", Some("Customer"), &s(&["Other"]), &[], &mut sc).unwrap_err();
        assert!(e.message.contains("one label") && e.message.contains("Other, Customer, _Entity"), "{}", e.message);
        // flat: as it always was
        let mut h = Hierarchy::default();
        assert_eq!(h.relabel("n", Some("Process"), &s(&["BpmnProcess"]), &s(&["Process"]), &mut sc).unwrap(), Move::To { label: Some("BpmnProcess".into()), added: 1, removed: 1 });
        assert_eq!(h.relabel("n", None, &s(&["A"]), &[], &mut sc).unwrap(), Move::To { label: Some("A".into()), added: 1, removed: 0 });
        assert_eq!(h.relabel("n", None, &[], &s(&["A"]), &mut sc).unwrap(), Move::Stay);
    }

    #[test]
    fn only_some_statements_read_the_hierarchy() {
        let n = |q: &str| needs(&cypher_syntax::parse(q).unwrap());
        for q in [
            "MATCH (n:_Entity:Customer) RETURN n",
            "MATCH (n:A|B) RETURN n",
            "MATCH (n) WHERE n:A RETURN n",
            "MATCH (n:_Entity) RETURN labels(n)[0] AS l",
            "MATCH (n:_Entity) WHERE $class IN labels(n) RETURN n",
            "MERGE (n:ANALYZER_OWNED:TABLE {id: 'x'}) RETURN n",
            "UNWIND $rows AS r CREATE (n:_Entity:Customer {k: r.k})",
            "MATCH (n:Customer) REMOVE n:Customer SET n:Client",
            "MATCH (a) WHERE EXISTS { MATCH (a)-[:R]->(:A:B) } RETURN a",
            "MATCH (a) FOREACH (x IN [1] | CREATE (:A:B {k: x}))",
            "MATCH (a) CALL { WITH a RETURN labels(a) AS l } RETURN l",
            "MATCH (a:A) RETURN a UNION MATCH (a) WHERE a:B RETURN a",
        ] {
            assert!(n(q), "{q}");
        }
        for q in ["MATCH (n:_Entity) RETURN n", "MATCH (n:A:A) RETURN n", "CREATE (n:A {k: 1})", "MATCH (a)-[:R|S]->(b) RETURN b", "RETURN 1 AS x"] {
            assert!(!n(q), "{q}");
        }
    }

    #[test]
    fn multi_label_writes_are_given_their_table() {
        let mut h = Hierarchy::of(&[], &[]);
        let q = cypher_syntax::parse(
            "MERGE (n:ANALYZER_OWNED:TABLE {id: 'x'}) WITH n FOREACH (c IN [1] | CREATE (:ANALYZER_OWNED:COLUMN {k: c})) \
             CREATE (m:A|B)",
        )
        .unwrap();
        let (q, sc) = store_all(&q, &mut h).unwrap();
        let Clause::Merge { pattern, .. } = &q.clauses[0] else { panic!() };
        let PatElem::Node(n) = &pattern.elems[0] else { panic!() };
        assert_eq!((n.labels.clone(), n.label_groups.clone()), (s(&["TABLE"]), vec![s(&["TABLE"])]));
        assert_eq!(sc.create, s(&["ANALYZER_OWNED", "TABLE", "COLUMN"]));
        assert_eq!(sc.inherit, vec![("TABLE".into(), "ANALYZER_OWNED".into()), ("COLUMN".into(), "ANALYZER_OWNED".into())]);
        // a choice is left for the printer to refuse
        let Clause::Create(ps) = &q.clauses[3] else { panic!() };
        let PatElem::Node(m) = &ps[0].elems[0] else { panic!() };
        assert_eq!(label_groups(m), vec![s(&["A", "B"])]);
    }
}
