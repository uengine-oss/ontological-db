//! Neo4j's procedures and `SHOW` commands, answered by the gateway.
//!
//! AGE's Cypher has no `CALL ns.proc()` and no `SHOW INDEXES`; what a client
//! asks of them is the graph's schema, which AGE keeps in its catalog
//! (`ag_label`) and the gateway's schema commands keep in PostgreSQL's
//! (`ddl.rs` makes an index on a label's table for `CREATE INDEX` /
//! `CREATE CONSTRAINT`). So a procedure is answered from those catalogs —
//! the reads DIRECTION §4.2 rule 6 allows — and what the statement does with
//! the answer stays Cypher: the rows go in as a parameter, and the clause
//! becomes `UNWIND $__og_procN AS label` (one column) or `UNWIND
//! $__og_procN AS __og_procrowN WITH …, __og_procrowN.name AS name, …`
//! (several), so `YIELD … WITH … WHERE … RETURN … ORDER BY` after it runs in
//! AGE as written (SUITES B5–B7, G9–G11; STUDIO G5a–G5d; ANALYZER §5.2).
//! A statement that is only the call, or the call and a `RETURN` of what it
//! yields, is answered from the rows directly ([`Direct`]).
//!
//! | Neo4j | answered from |
//! |---|---|
//! | `db.labels()`, `db.relationshipTypes()` | `ag_label` |
//! | `db.propertyKeys()` | Cypher: `keys()` of every node and relationship |
//! | `SHOW INDEXES`, `SHOW CONSTRAINTS` | the indexes `ddl.rs` made on the label tables (`pg_index`, `pg_get_indexdef`) |
//! | `dbms.components()` | one row, what the server's agent says (`Neo4j Kernel`, `4.4.0`, `community`) |
//! | `db.awaitIndexes(n)`, `db.awaitIndex(name, n)` | nothing to wait for: `ddl.rs` makes indexes in the statement |
//! | `apoc.meta.schema()` | `ag_label`, the label hierarchy, a Cypher count per label and a sample of 100 elements per label (approximate) |
//! | `apoc.neighbors.tohop(n, '>', d)` | not a catalog read: a variable-length `MATCH` ([`tohop`]) |
//!
//! Any other procedure is `Neo.ClientError.Procedure.ProcedureNotFound`, as
//! Neo4j answers a name it has not registered. Vector and full-text search
//! (`db.index.vector.queryNodes`, `db.index.fulltext.queryNodes`) are among
//! them: the gateway would have to read the label tables in SQL for them,
//! which it does not do (DIRECTION §4.2 rule 7).

use super::agtype::Ag;
use super::cypher::{ident, scope_after, FRESH};
use super::hierarchy::{self, Hierarchy};
use super::split::Exec;
use super::{sql_lit, Refusal};
use cypher_syntax::ast::*;
use serde_json::{json, Map, Value as Json};
use std::collections::BTreeMap;

/// What a procedure's rows come from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    Labels,
    RelationshipTypes,
    PropertyKeys,
    Indexes,
    Constraints,
    Components,
    MetaSchema,
}

/// `SHOW INDEXES` columns: Neo4j 5's and 4.4's (`uniqueness`), in Neo4j's order.
const INDEX_COLUMNS: &[&str] = &[
    "id", "name", "state", "populationPercent", "uniqueness", "type", "entityType", "labelsOrTypes", "properties",
    "indexProvider", "owningConstraint", "lastRead", "readCount", "trackedSince", "options", "failureMessage",
    "createStatement",
];
const CONSTRAINT_COLUMNS: &[&str] =
    &["id", "name", "type", "entityType", "labelsOrTypes", "properties", "ownedIndex", "propertyType", "options", "createStatement"];

impl Source {
    /// Every column it yields.
    pub fn columns(self) -> &'static [&'static str] {
        match self {
            Source::Labels => &["label"],
            Source::RelationshipTypes => &["relationshipType"],
            Source::PropertyKeys => &["propertyKey"],
            Source::Indexes => INDEX_COLUMNS,
            Source::Constraints => CONSTRAINT_COLUMNS,
            Source::Components => &["name", "versions", "edition"],
            Source::MetaSchema => &["value"],
        }
    }
}

enum Proc {
    Read(Source),
    /// Returns nothing and changes no row (Neo4j's VOID procedures).
    Void,
    ToHop,
}

fn which(name: &str) -> Option<Proc> {
    Some(match name.to_ascii_lowercase().as_str() {
        "db.labels" => Proc::Read(Source::Labels),
        "db.relationshiptypes" => Proc::Read(Source::RelationshipTypes),
        "db.propertykeys" => Proc::Read(Source::PropertyKeys),
        // the parser's names for SHOW INDEXES / SHOW CONSTRAINTS
        "show.indexes" => Proc::Read(Source::Indexes),
        "show.constraints" => Proc::Read(Source::Constraints),
        "dbms.components" => Proc::Read(Source::Components),
        "apoc.meta.schema" => Proc::Read(Source::MetaSchema),
        "db.awaitindexes" | "db.awaitindex" => Proc::Void,
        "apoc.neighbors.tohop" => Proc::ToHop,
        _ => return None,
    })
}

/// A catalog read a statement needs before it runs: its rows go into the
/// parameter `param`, as a list of the one column's values (`single`) or of
/// maps of the columns `cols`.
#[derive(Clone, Debug, PartialEq)]
pub struct Read {
    pub source: Source,
    pub param: String,
    pub cols: Vec<String>,
    pub single: bool,
}

/// A statement that is only a procedure (and a `RETURN` of what it yields):
/// its rows are the read's, each result column one of its columns. `read:
/// None` — a VOID procedure alone: no rows.
#[derive(Clone, Debug, PartialEq)]
pub struct Direct {
    pub read: Option<Read>,
    /// The read's column each result column is.
    pub out: Vec<String>,
}

/// A statement with its procedure calls taken out.
pub struct Rewritten {
    pub query: Query,
    pub reads: Vec<Read>,
    pub direct: Option<(Direct, Vec<String>)>,
}

fn not_found(name: &str) -> Refusal {
    let lower = name.to_ascii_lowercase();
    let why = if lower.starts_with("db.index.vector.") || lower.starts_with("db.index.fulltext.") {
        " On the AGE backend vector and full-text search are not done by the gateway: it would have to read the label \
         tables in SQL (bolt/README.md)."
    } else {
        ""
    };
    Refusal {
        code: "Neo.ClientError.Procedure.ProcedureNotFound".into(),
        message: format!(
            "There is no procedure with the name `{name}` registered for this database instance. Please ensure you've \
             spelled the procedure name correctly and that the procedure is properly deployed.{why} The AGE backend \
             answers db.labels, db.relationshipTypes, db.propertyKeys, db.awaitIndexes, db.awaitIndex, \
             dbms.components, apoc.meta.schema, apoc.neighbors.tohop and SHOW INDEXES / SHOW CONSTRAINTS."
        ),
    }
}

fn syntax(message: String) -> Refusal {
    Refusal::syntax(message)
}

fn argument(message: String) -> Refusal {
    Refusal { code: "Neo.ClientError.Statement.ArgumentError".into(), message }
}

/// Does the statement call a procedure anywhere the rewrite looks?
fn has_call(q: &Query) -> bool {
    q.clauses.iter().any(|c| match c {
        Clause::Call { .. } => true,
        Clause::Subquery { body, .. } => has_call(body),
        _ => false,
    }) || q.union.as_ref().is_some_and(|(_, t)| has_call(t))
}

/// The yielded (column, alias) pairs, checked against what the source has;
/// all its columns when there is no YIELD.
fn yields_of(src: Source, yields: &[(String, String)]) -> Result<Vec<(String, String)>, Refusal> {
    if yields.is_empty() {
        return Ok(src.columns().iter().map(|c| (c.to_string(), c.to_string())).collect());
    }
    for (c, _) in yields {
        if !src.columns().contains(&c.as_str()) {
            return Err(syntax(format!("Unknown procedure output: `{c}`")));
        }
    }
    Ok(yields.to_vec())
}

struct State<'a> {
    params: Option<&'a Map<String, Json>>,
    reads: Vec<Read>,
}

impl State<'_> {
    fn fresh(&self, what: &str) -> String {
        format!("{FRESH}{what}{}", self.reads.len() + 1)
    }
}

/// Take the procedure calls out of a statement. `None`: it has none.
pub fn rewrite(q: &Query, params: Option<&Map<String, Json>>) -> Result<Option<Rewritten>, Refusal> {
    if !has_call(q) {
        return Ok(None);
    }
    let mut st = State { params, reads: Vec::new() };
    // A statement that is only the call (and a RETURN of what it yields).
    if q.union.is_none() {
        if let Some(Clause::Call { name, yields, .. }) = q.clauses.first() {
            match which(name) {
                Some(Proc::Void) if q.clauses.len() == 1 => {
                    return Ok(Some(Rewritten {
                        query: q.clone(),
                        reads: Vec::new(),
                        direct: Some((Direct { read: None, out: Vec::new() }, Vec::new())),
                    }));
                }
                Some(Proc::Read(src)) => {
                    let ys = yields_of(src, yields)?;
                    let simple = match &q.clauses[1..] {
                        [] => Some(ys.iter().map(|(c, a)| (c.clone(), a.clone())).collect::<Vec<_>>()),
                        [Clause::Return(p)] if !p.distinct && p.order.is_empty() && p.skip.is_none() && p.limit.is_none() => {
                            let mut out = Vec::new();
                            for it in &p.items {
                                match &it.expr {
                                    Expr::Var(v) => match ys.iter().find(|(_, a)| a == v) {
                                        Some((c, _)) => out.push((c.clone(), it.alias.clone().unwrap_or_else(|| v.clone()))),
                                        None => {
                                            out.clear();
                                            break;
                                        }
                                    },
                                    _ => {
                                        out.clear();
                                        break;
                                    }
                                }
                            }
                            if out.is_empty() {
                                None
                            } else {
                                Some(out)
                            }
                        }
                        _ => None,
                    };
                    if let Some(out) = simple {
                        let read = Read {
                            source: src,
                            param: String::new(),
                            cols: src.columns().iter().map(|c| c.to_string()).collect(),
                            single: false,
                        };
                        let (cols, names): (Vec<String>, Vec<String>) = out.into_iter().unzip();
                        return Ok(Some(Rewritten { query: q.clone(), reads: Vec::new(), direct: Some((Direct { read: Some(read), out: cols }, names)) }));
                    }
                }
                _ => {}
            }
        }
    }
    let mut query = rewrite_query(q, Vec::new(), &mut st)?;
    // A statement that ends with the call, or with SHOW's YIELD … WHERE: what
    // it yields is what it returns (Neo4j's standalone call).
    if !query.clauses.iter().any(|c| matches!(c, Clause::Return(_))) && q.union.is_none() && !super::cypher::is_write(q) {
        let last = q.clauses.iter().rev().find_map(|c| match c {
            Clause::Call { name, yields, .. } => Some((name, yields)),
            _ => None,
        });
        if let Some((name, yields)) = last {
            let aliases: Vec<String> = match which(name) {
                Some(Proc::Read(src)) => yields_of(src, yields)?.into_iter().map(|(_, a)| a).collect(),
                Some(Proc::ToHop) => vec![yields.iter().find(|(c, _)| c == "node").map(|(_, a)| a.clone()).unwrap_or_else(|| "node".into())],
                _ => Vec::new(),
            };
            if !aliases.is_empty() {
                query.clauses.push(Clause::Return(Projection {
                    items: aliases.iter().map(|a| ReturnItem { expr: Expr::Var(a.clone()), alias: None }).collect(),
                    ..Projection::default()
                }));
            }
        }
    }
    Ok(Some(Rewritten { query, reads: st.reads, direct: None }))
}

fn rewrite_query(q: &Query, scope0: Vec<String>, st: &mut State) -> Result<Query, Refusal> {
    let clauses = rewrite_clauses(&q.clauses, scope0.clone(), st)?;
    let union = match &q.union {
        Some((all, tail)) => Some((*all, Box::new(rewrite_query(tail, scope0, st)?))),
        None => None,
    };
    Ok(Query { clauses, union })
}

fn rewrite_clauses(cs: &[Clause], mut scope: Vec<String>, st: &mut State) -> Result<Vec<Clause>, Refusal> {
    let mut out = Vec::new();
    for c in cs {
        match c {
            Clause::Call { name, args, yields } => match which(name).ok_or_else(|| not_found(name))? {
                Proc::Void => {}
                Proc::Read(src) => {
                    let ys = yields_of(src, yields)?;
                    let param = st.fresh("proc");
                    let single = ys.len() == 1;
                    if single {
                        out.push(Clause::Unwind { expr: Expr::Param(param.clone()), alias: ys[0].1.clone() });
                    } else {
                        let row = st.fresh("procrow");
                        out.push(Clause::Unwind { expr: Expr::Param(param.clone()), alias: row.clone() });
                        let mut items: Vec<ReturnItem> = scope.iter().map(|v| ReturnItem { expr: Expr::Var(v.clone()), alias: None }).collect();
                        for (col, alias) in &ys {
                            items.push(ReturnItem { expr: Expr::Prop(Box::new(Expr::Var(row.clone())), col.clone()), alias: Some(alias.clone()) });
                        }
                        out.push(Clause::With { proj: Projection { items, ..Projection::default() }, where_: None });
                    }
                    st.reads.push(Read { source: src, param, cols: ys.into_iter().map(|(c, _)| c).collect(), single });
                }
                Proc::ToHop => out.extend(tohop(args, yields, &scope, st.params)?),
            },
            Clause::Subquery { scope: sc, body, optional } => {
                let sees = match sc {
                    Some(names) if names.iter().any(|n| n == "*") => scope.clone(),
                    Some(names) => names.clone(),
                    None => Vec::new(),
                };
                out.push(Clause::Subquery { scope: sc.clone(), body: Box::new(rewrite_query(body, sees, st)?), optional: *optional });
            }
            other => out.push(other.clone()),
        }
        scope = scope_after(&scope, c);
    }
    Ok(out)
}

// ------------------------------------------------------------- tohop

/// `CALL apoc.neighbors.tohop(n, '>', d) YIELD node AS m` (STUDIO G5c, the
/// neighbourhood view): the nodes reached from `n` over 1 to `d`
/// relationships of the types and direction the second argument names, each
/// once, `n` itself not among them (APOC's answer). Sent as
///
/// `MATCH (n)-[:T*1..d]->(m) WHERE id(m) <> id(n) WITH DISTINCT <what is in scope>, m`
///
/// The second argument is APOC's relationship filter: `'>'` (any type,
/// outgoing), `'<'`, `''` or null (both ways), `'KNOWS>'`, `'<KNOWS|LIKES'`
/// … — one direction for all its types, since one pattern has one. `d` is a
/// literal or a parameter, known before the statement is printed. With no
/// type or several and more than 3 hops, the pattern is one the gateway
/// walks itself (`reach.rs`: the MATCH is followed by DISTINCT), or refused
/// as reach.rs refuses. Two rows that are the same in every variable before
/// the call come out as one — DISTINCT does not tell them apart, where APOC
/// is called once for each.
fn tohop(args: &[Expr], yields: &[(String, String)], scope: &[String], params: Option<&Map<String, Json>>) -> Result<Vec<Clause>, Refusal> {
    let [start, filter, depth] = args else {
        return Err(syntax(format!("apoc.neighbors.tohop takes 3 arguments (node, relationship filter, distance), not {}", args.len())));
    };
    let Expr::Var(n) = start else {
        return Err(syntax("apoc.neighbors.tohop: the start node must be a variable on the AGE backend".into()));
    };
    if !scope.contains(n) {
        return Err(syntax(format!("Variable `{n}` not defined")));
    }
    let spec: Option<String> = match filter {
        Expr::Lit(Lit::Str(s)) => Some(s.clone()),
        Expr::Lit(Lit::Null) => None,
        Expr::Param(p) => match params.and_then(|m| m.get(p)) {
            Some(Json::String(s)) => Some(s.clone()),
            Some(Json::Null) | None => None,
            Some(other) => return Err(argument(format!("apoc.neighbors.tohop: the relationship filter must be a string, not {other}"))),
        },
        _ => return Err(syntax("apoc.neighbors.tohop: the relationship filter must be a literal or a parameter on the AGE backend".into())),
    };
    let (types, dir) = rel_filter(spec.as_deref().unwrap_or(""))?;
    let d = match depth {
        Expr::Lit(Lit::Int(i)) => Some(*i),
        Expr::Param(p) => match params.and_then(|m| m.get(p)) {
            Some(j) => j.as_i64(),
            None => return Err(Refusal { code: "Neo.ClientError.Statement.ParameterMissing".into(), message: format!("Expected parameter(s): {p}") }),
        },
        _ => return Err(syntax("apoc.neighbors.tohop: the distance must be a literal or a parameter on the AGE backend".into())),
    };
    let d = d.ok_or_else(|| argument("apoc.neighbors.tohop: the distance must be an integer".into()))?;
    if !(1..=30).contains(&d) {
        return Err(argument(format!("apoc.neighbors.tohop: a distance of {d} is not run on the AGE backend (1 to 30)")));
    }
    for (c, _) in yields {
        if c != "node" {
            return Err(syntax(format!("Unknown procedure output: `{c}`")));
        }
    }
    let m = yields.first().map(|(_, a)| a.clone()).unwrap_or_else(|| "node".into());
    if scope.contains(&m) {
        return Err(syntax(format!("Variable `{m}` already declared")));
    }
    let node = |v: &str| PatElem::Node(NodePat { var: Some(v.to_string()), labels: Vec::new(), label_groups: Vec::new(), props: Vec::new() });
    let rel = PatElem::Rel(RelPat { var: None, types, dir, props: Vec::new(), range: Some((1, d as u32)) });
    let id = |v: &str| Expr::Func { name: "id".into(), args: vec![Expr::Var(v.to_string())], distinct: false };
    let mut items: Vec<ReturnItem> = scope.iter().map(|v| ReturnItem { expr: Expr::Var(v.clone()), alias: None }).collect();
    items.push(ReturnItem { expr: Expr::Var(m.clone()), alias: None });
    Ok(vec![
        Clause::Match {
            patterns: vec![Pattern { path_var: None, elems: vec![node(n), rel, node(&m)] }],
            optional: false,
            where_: Some(Expr::Binary(BinOp::Ne, Box::new(id(&m)), Box::new(id(n)))),
        },
        Clause::With { proj: Projection { items, distinct: true, ..Projection::default() }, where_: None },
    ])
}

/// APOC's relationship filter (`'KNOWS>|<LIKES'`): the types (empty: any)
/// and the one direction.
fn rel_filter(spec: &str) -> Result<(Vec<String>, Dir), Refusal> {
    let mut types: Vec<String> = Vec::new();
    let mut any = false;
    let mut dir: Option<Dir> = None;
    let parts: Vec<&str> = if spec.trim().is_empty() { vec![""] } else { spec.split('|').collect() };
    for part in parts {
        let p = part.trim();
        let (p, d) = if let Some(t) = p.strip_prefix('<') {
            (t, Dir::In)
        } else if let Some(t) = p.strip_suffix('>') {
            (t, Dir::Out)
        } else {
            (p, Dir::Both)
        };
        if p.contains('<') || p.contains('>') {
            return Err(argument(format!("apoc.neighbors.tohop: `{part}` is not a relationship filter")));
        }
        match &dir {
            None => dir = Some(d),
            Some(x) if *x == d => {}
            Some(_) => {
                return Err(syntax(format!(
                    "apoc.neighbors.tohop: '{spec}' names more than one direction; the AGE backend walks one (a pattern has one direction)"
                )))
            }
        }
        if p.is_empty() {
            any = true;
        } else if !types.iter().any(|t| t == p) {
            types.push(p.to_string());
        }
    }
    if any {
        types.clear();
    }
    Ok((types, dir.unwrap_or(Dir::Both)))
}

// ------------------------------------------------------------- the reads

/// The vertex or edge labels of the graph, sorted.
fn labels_sql(graph: &str, kind: char) -> String {
    format!(
        "SELECT l.name::text FROM ag_catalog.ag_label l JOIN ag_catalog.ag_graph g ON g.graphid = l.graph \
         WHERE g.name = {} AND l.kind = '{kind}' AND l.name NOT IN ('_ag_label_vertex', '_ag_label_edge') ORDER BY 1",
        sql_lit(graph)
    )
}

/// The expression indexes on the graph's label tables, one row per key
/// column: index name, label, kind, unique, valid, its oid, the column's
/// definition. Those are the indexes `ddl.rs` makes; AGE's own (on `id`,
/// `start_id`, `end_id`) are plain columns and left out.
pub fn indexes_sql(graph: &str) -> String {
    format!(
        "SELECT c.relname::text, l.name::text, l.kind::text, x.indisunique::text, x.indisvalid::text, c.oid::text, \
         pg_catalog.pg_get_indexdef(x.indexrelid, k, true) \
         FROM ag_catalog.ag_label l \
         JOIN ag_catalog.ag_graph g ON g.graphid = l.graph \
         JOIN pg_catalog.pg_index x ON x.indrelid = l.relation::oid \
         JOIN pg_catalog.pg_class c ON c.oid = x.indexrelid \
         CROSS JOIN LATERAL pg_catalog.generate_series(1, x.indnkeyatts::int) AS k \
         WHERE g.name = {} AND x.indexprs IS NOT NULL AND l.name NOT IN ('_ag_label_vertex', '_ag_label_edge') \
         ORDER BY c.relname, k",
        sql_lit(graph)
    )
}

/// The property key of an index column `ddl.rs` made —
/// `agtype_access_operator(VARIADIC ARRAY[properties, '"id"'::agtype])`,
/// however PostgreSQL prints it back. `None` for any other expression.
pub fn index_key(def: &str) -> Option<String> {
    let at = def.find("agtype_access_operator(")?;
    let rest = &def[at..];
    let lit = rest.find("properties, '")? + "properties, '".len();
    let mut s = String::new();
    let mut chars = rest[lit..].char_indices().peekable();
    let mut end = None;
    while let Some((i, c)) = chars.next() {
        if c == '\'' {
            if chars.peek().is_some_and(|(_, n)| *n == '\'') {
                s.push('\'');
                chars.next();
                continue;
            }
            end = Some(lit + i + 1);
            break;
        }
        s.push(c);
    }
    let after = &rest[end?..];
    let after = after.strip_prefix("::")?;
    let after = after.trim_start_matches("ag_catalog.").strip_prefix("agtype")?;
    if !after.trim_start().starts_with(']') {
        return None;
    }
    match serde_json::from_str::<Json>(&s).ok()? {
        Json::String(k) => Some(k),
        _ => None,
    }
}

/// One index `ddl.rs` made, as read back.
#[derive(Clone, Debug, PartialEq)]
pub struct Index {
    pub name: String,
    pub label: String,
    pub edge: bool,
    pub unique: bool,
    pub valid: bool,
    pub id: i64,
    pub props: Vec<String>,
}

/// [`indexes_sql`]'s rows, grouped; an index with a column that is not a
/// property access is not one of the gateway's, and is left out.
pub fn indexes_from_rows(rows: &[Vec<Option<String>>]) -> Vec<Index> {
    let mut out: Vec<Index> = Vec::new();
    let mut bad: Vec<String> = Vec::new();
    for r in rows {
        let get = |i: usize| r.get(i).cloned().flatten().unwrap_or_default();
        let name = get(0);
        let key = index_key(&get(6));
        match (out.iter_mut().find(|x| x.name == name), key) {
            (_, None) => bad.push(name),
            (Some(x), Some(k)) => x.props.push(k),
            (None, Some(k)) => out.push(Index {
                name,
                label: get(1),
                edge: get(2) == "e",
                unique: get(3) == "t" || get(3) == "true",
                valid: get(4) == "t" || get(4) == "true",
                id: get(5).parse().unwrap_or(0),
                props: vec![k],
            }),
        }
    }
    out.retain(|x| !bad.contains(&x.name));
    out
}

fn bq(s: &str) -> String {
    format!("`{}`", s.replace('`', "``"))
}

fn create_statement(ix: &Index) -> String {
    let (pat, v) = if ix.edge { (format!("()-[r:{}]-()", bq(&ix.label)), "r") } else { (format!("(n:{})", bq(&ix.label)), "n") };
    let props = ix.props.iter().map(|p| format!("{v}.{}", bq(p))).collect::<Vec<_>>().join(", ");
    if ix.unique {
        format!("CREATE CONSTRAINT {} FOR {pat} REQUIRE ({props}) IS UNIQUE", bq(&ix.name))
    } else {
        format!("CREATE RANGE INDEX {} FOR {pat} ON ({props})", bq(&ix.name))
    }
}

fn options() -> Json {
    json!({"indexProvider": "range-1.0", "indexConfig": {}})
}

/// `SHOW INDEXES` rows: every index on a label table the gateway made,
/// uniqueness constraints' own included (Neo4j lists those too, with
/// `owningConstraint`). `type` is `RANGE`, what Neo4j 5 calls a b-tree on
/// properties. Vector and full-text indexes are accepted by `CREATE` and
/// not made (`ddl.rs`), so there are none to list.
pub fn index_rows(ixs: &[Index]) -> Vec<Map<String, Json>> {
    ixs.iter()
        .map(|ix| {
            let m = json!({
                "id": ix.id,
                "name": ix.name,
                "state": if ix.valid { "ONLINE" } else { "FAILED" },
                "populationPercent": 100.0,
                "uniqueness": if ix.unique { "UNIQUE" } else { "NONUNIQUE" },
                "type": "RANGE",
                "entityType": if ix.edge { "RELATIONSHIP" } else { "NODE" },
                "labelsOrTypes": [ix.label],
                "properties": ix.props,
                "indexProvider": "range-1.0",
                "owningConstraint": if ix.unique { Json::String(ix.name.clone()) } else { Json::Null },
                "lastRead": null,
                "readCount": null,
                "trackedSince": null,
                "options": options(),
                "failureMessage": if ix.valid { "" } else { "the index is not valid in PostgreSQL (pg_index.indisvalid)" },
                "createStatement": create_statement(ix),
            });
            m.as_object().cloned().unwrap_or_default()
        })
        .collect()
}

/// `SHOW CONSTRAINTS` rows: the unique indexes. A `NODE KEY` constraint is
/// one too (`ddl.rs` makes a unique index for it) and reads back as
/// `UNIQUENESS`; `IS NOT NULL` constraints are accepted and not made, so are
/// not listed.
pub fn constraint_rows(ixs: &[Index]) -> Vec<Map<String, Json>> {
    ixs.iter()
        .filter(|ix| ix.unique)
        .map(|ix| {
            let m = json!({
                "id": ix.id,
                "name": ix.name,
                "type": if ix.edge { "RELATIONSHIP_UNIQUENESS" } else { "UNIQUENESS" },
                "entityType": if ix.edge { "RELATIONSHIP" } else { "NODE" },
                "labelsOrTypes": [ix.label],
                "properties": ix.props,
                "ownedIndex": ix.name,
                "propertyType": null,
                "options": options(),
                "createStatement": create_statement(ix),
            });
            m.as_object().cloned().unwrap_or_default()
        })
        .collect()
}

/// What `dbms.components()` answers: the server the gateway says it is
/// (`HELLO`'s `Neo4j/4.4.0`).
pub fn component_rows() -> Vec<Map<String, Json>> {
    vec![json!({"name": "Neo4j Kernel", "versions": ["4.4.0"], "edition": "community"}).as_object().cloned().unwrap_or_default()]
}

fn one_col(col: &str, vals: impl IntoIterator<Item = String>) -> Vec<Map<String, Json>> {
    vals.into_iter()
        .map(|v| {
            let mut m = Map::new();
            m.insert(col.into(), Json::String(v));
            m
        })
        .collect()
}

/// The type APOC gives a value.
fn apoc_type(a: &Ag) -> Option<&'static str> {
    Some(match a {
        Ag::Null => return None,
        Ag::Bool(_) => "BOOLEAN",
        Ag::Int(_) => "INTEGER",
        Ag::Float(_) | Ag::Numeric(_) => "FLOAT",
        Ag::Str(_) => "STRING",
        Ag::List(_) => "LIST",
        Ag::Map(_) => "MAP",
        Ag::Vertex { .. } => "NODE",
        Ag::Edge { .. } => "RELATIONSHIP",
        Ag::Path(_) => "PATH",
    })
}

/// Per property key, how often each type was seen.
type Seen = BTreeMap<String, BTreeMap<&'static str, usize>>;

fn sample_into(seen: &mut Seen, rows: &[Vec<Ag>]) {
    for r in rows {
        if let Some(Ag::Map(kv)) = r.first() {
            for (k, v) in kv {
                let e = seen.entry(k.clone()).or_default();
                if let Some(t) = apoc_type(v) {
                    *e.entry(t).or_default() += 1;
                }
            }
        }
    }
}

/// The most frequent type, `STRING` when only nulls were seen.
fn type_of(counts: &BTreeMap<&'static str, usize>) -> &'static str {
    counts.iter().max_by_key(|(_, n)| **n).map(|(t, _)| *t).unwrap_or("STRING")
}

/// How many elements each label (vertex: and the labels below it) has.
fn count_by(rows: &[Vec<Ag>]) -> BTreeMap<String, i64> {
    let mut out = BTreeMap::new();
    for r in rows {
        if let (Some(Ag::Str(l)), Some(Ag::Int(n))) = (r.first(), r.get(1)) {
            out.insert(l.clone(), *n);
        }
    }
    out
}

/// How many elements of each label the sample of `apoc.meta.schema()` reads.
pub const META_SAMPLE: usize = 100;

/// `apoc.meta.schema()`'s one row (STUDIO G5d). Approximate where APOC
/// is not: a property's `type` is the one seen most in a sample of
/// [`META_SAMPLE`] elements stored under each label (a label's are its own
/// and those of the labels whose tables inherit it, the nodes that have it);
/// dates are text here and read as `STRING`; `relationships` of a node
/// label is left empty; `existence` is always false (the gateway does not
/// enforce `IS NOT NULL`). `count` is exact (a Cypher count per label).
/// `indexed` / `unique` come from the indexes the gateway made.
fn meta_schema(x: &mut dyn Exec) -> Result<Vec<Map<String, Json>>, Refusal> {
    let h: Hierarchy = hierarchy::read(x)?;
    let ixs = indexes_from_rows(&x.sql(&indexes_sql(&x.graph()))?);
    let vcount = count_by(&x.cypher("MATCH (n) RETURN label(n), count(*)", 2, &json!({}))?);
    let ecount = count_by(&x.cypher("MATCH ()-[r]->() RETURN type(r), count(*)", 2, &json!({}))?);
    let mut stored: BTreeMap<String, Seen> = BTreeMap::new();
    for l in &h.vertex {
        let text = format!(
            "MATCH (n:{}) WHERE label(n) = {} WITH n LIMIT {META_SAMPLE} RETURN properties(n)",
            ident(l),
            super::cypher::str_lit(l)
        );
        let rows = x.cypher(&text, 1, &json!({}))?;
        sample_into(stored.entry(l.clone()).or_default(), &rows);
    }
    let props_json = |seen: &Seen, label: &str, edge: bool| -> Json {
        let mut m = Map::new();
        for (k, counts) in seen {
            let ty = type_of(counts);
            let on = |ix: &&Index| ix.edge == edge && ix.label == label && ix.props.iter().any(|p| p == k);
            m.insert(
                k.clone(),
                json!({
                    "type": ty,
                    "array": ty == "LIST",
                    "indexed": ixs.iter().any(|ix| on(&ix)),
                    "unique": ixs.iter().any(|ix| ix.unique && on(&ix)),
                    "existence": false,
                }),
            );
        }
        Json::Object(m)
    };
    let mut value = Map::new();
    for l in &h.vertex {
        let below = h.down(l);
        let mut seen = Seen::new();
        for b in &below {
            if let Some(s) = stored.get(b) {
                for (k, counts) in s {
                    let e = seen.entry(k.clone()).or_default();
                    for (t, n) in counts {
                        *e.entry(t).or_default() += n;
                    }
                }
            }
        }
        let count: i64 = below.iter().map(|b| vcount.get(b).copied().unwrap_or(0)).sum();
        let others: Vec<String> = h.up(l).into_iter().filter(|x| x != l).collect();
        value.insert(
            l.clone(),
            json!({"type": "node", "count": count, "labels": others, "properties": props_json(&seen, l, false), "relationships": {}}),
        );
    }
    for t in &h.edge {
        let text = format!("MATCH ()-[r:{}]->() WITH r LIMIT {META_SAMPLE} RETURN properties(r)", ident(t));
        let rows = x.cypher(&text, 1, &json!({}))?;
        let mut seen = Seen::new();
        sample_into(&mut seen, &rows);
        value.insert(
            t.clone(),
            json!({"type": "relationship", "count": ecount.get(t).copied().unwrap_or(0), "properties": props_json(&seen, t, true)}),
        );
    }
    let mut row = Map::new();
    row.insert("value".into(), Json::Object(value));
    Ok(vec![row])
}

/// The rows of a source, every column.
pub fn rows(x: &mut dyn Exec, src: Source) -> Result<Vec<Map<String, Json>>, Refusal> {
    let graph = x.graph();
    let first = |rows: Vec<Vec<Option<String>>>| rows.into_iter().filter_map(|r| r.into_iter().next().flatten()).collect::<Vec<_>>();
    Ok(match src {
        Source::Labels => one_col("label", first(x.sql(&labels_sql(&graph, 'v'))?)),
        Source::RelationshipTypes => one_col("relationshipType", first(x.sql(&labels_sql(&graph, 'e'))?)),
        Source::PropertyKeys => {
            // read through Cypher, not the label tables' SQL (DIRECTION §4.2 rule 4)
            let mut keys: Vec<String> = Vec::new();
            for text in ["MATCH (n) UNWIND keys(n) AS k RETURN DISTINCT k", "MATCH ()-[r]->() UNWIND keys(r) AS k RETURN DISTINCT k"] {
                for r in x.cypher(text, 1, &json!({}))? {
                    if let Some(Ag::Str(k)) = r.into_iter().next() {
                        if !keys.contains(&k) {
                            keys.push(k);
                        }
                    }
                }
            }
            keys.sort();
            one_col("propertyKey", keys)
        }
        Source::Indexes => index_rows(&indexes_from_rows(&x.sql(&indexes_sql(&graph))?)),
        Source::Constraints => constraint_rows(&indexes_from_rows(&x.sql(&indexes_sql(&graph))?)),
        Source::Components => component_rows(),
        Source::MetaSchema => meta_schema(x)?,
    })
}

/// The read's rows as its parameter.
pub fn as_param(read: &Read, rows: &[Map<String, Json>]) -> Json {
    Json::Array(
        rows.iter()
            .map(|r| {
                if read.single {
                    r.get(&read.cols[0]).cloned().unwrap_or(Json::Null)
                } else {
                    let mut m = Map::new();
                    for c in &read.cols {
                        m.insert(c.clone(), r.get(c).cloned().unwrap_or(Json::Null));
                    }
                    Json::Object(m)
                }
            })
            .collect(),
    )
}

/// A JSON value as the agtype the rest of the gateway turns into Bolt.
pub fn json_to_ag(v: &Json) -> Ag {
    match v {
        Json::Null => Ag::Null,
        Json::Bool(b) => Ag::Bool(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) if !n.is_f64() => Ag::Int(i),
            _ => Ag::Float(n.as_f64().unwrap_or(f64::NAN)),
        },
        Json::String(s) => Ag::Str(s.clone()),
        Json::Array(xs) => Ag::List(xs.iter().map(json_to_ag).collect()),
        Json::Object(m) => Ag::Map(m.iter().map(|(k, v)| (k.clone(), json_to_ag(v))).collect()),
    }
}

/// A [`Direct`] statement's result rows.
pub fn direct(x: &mut dyn Exec, d: &Direct) -> Result<Vec<Vec<Ag>>, Refusal> {
    let Some(read) = &d.read else { return Ok(Vec::new()) };
    Ok(rows(x, read.source)?
        .iter()
        .map(|r| d.out.iter().map(|c| json_to_ag(r.get(c).unwrap_or(&Json::Null))).collect())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rw(q: &str) -> Rewritten {
        rewrite(&cypher_syntax::parse(q).unwrap(), None).unwrap().unwrap()
    }

    fn rw_p(q: &str, params: Json) -> Result<Rewritten, Refusal> {
        rewrite(&cypher_syntax::parse(q).unwrap(), params.as_object()).map(Option::unwrap)
    }

    fn printed(r: &Rewritten) -> String {
        super::super::cypher::Printer::new(None, "2026-11-18T00:00:00+00:00".into()).query(&r.query).unwrap()
    }

    #[test]
    fn a_call_alone_is_answered_from_the_rows() {
        let r = rw("CALL db.labels()");
        let (d, names) = r.direct.unwrap();
        assert_eq!((d.read.unwrap().source, d.out, names), (Source::Labels, vec!["label".to_string()], vec!["label".to_string()]));
        let (d, names) = rw("CALL db.propertyKeys() YIELD propertyKey AS k RETURN k AS key").direct.unwrap();
        assert_eq!((d.out, names), (vec!["propertyKey".to_string()], vec!["key".to_string()]));
        // SHOW, bare: the parser's columns
        let (d, names) = rw("SHOW INDEXES").direct.unwrap();
        assert_eq!(names, vec!["id", "name", "state", "type", "entityType", "labelsOrTypes", "properties"]);
        assert_eq!(d.read.unwrap().source, Source::Indexes);
        let (_, names) = rw("SHOW CONSTRAINTS YIELD name").direct.unwrap();
        assert_eq!(names, vec!["name"]);
        // STUDIO G5a
        let (d, names) = rw("CALL dbms.components() YIELD name, versions, edition RETURN name, versions, edition").direct.unwrap();
        assert_eq!((d.read.unwrap().source, names.len()), (Source::Components, 3));
        // B7: nothing to wait for
        let (d, names) = rw("CALL db.awaitIndexes(30)").direct.unwrap();
        assert!(d.read.is_none() && names.is_empty());
    }

    #[test]
    fn what_follows_the_call_stays_cypher() {
        // SUITES B5 / G9
        let r = rw("CALL db.labels() YIELD label WITH label WHERE label STARTS WITH $p RETURN label ORDER BY label");
        assert!(r.direct.is_none());
        assert_eq!(r.reads, vec![Read { source: Source::Labels, param: "__og_proc1".into(), cols: vec!["label".into()], single: true }]);
        assert_eq!(printed(&r), "UNWIND $__og_proc1 AS label WITH label WHERE label STARTS WITH $p RETURN label ORDER BY label");
        // ANALYZER §5.2: SHOW … YIELD … WHERE … RETURN
        let r = rw("SHOW INDEXES YIELD name, state WHERE name = 'analyzer_search_vec' RETURN state");
        assert_eq!(
            printed(&r),
            "UNWIND $__og_proc1 AS __og_procrow1 WITH __og_procrow1.name AS name, __og_procrow1.state AS state \
             WITH name AS name, state AS state WHERE name = 'analyzer_search_vec' RETURN state"
        );
        assert_eq!(r.reads[0].cols, vec!["name", "state"]);
        // STUDIO G5b; and YIELD … WHERE without RETURN returns what it yields
        let r = rw("SHOW CONSTRAINTS YIELD name, labelsOrTypes WHERE name STARTS WITH $prefix RETURN name, labelsOrTypes");
        assert!(printed(&r).ends_with("WHERE name STARTS WITH $prefix RETURN name, labelsOrTypes"), "{}", printed(&r));
        let r = rw("SHOW INDEXES YIELD name, options WHERE name = $name");
        assert!(printed(&r).ends_with("WHERE name = $name RETURN name, options"), "{}", printed(&r));
        assert_eq!(super::super::cypher::columns(&r.query), vec!["name", "options"]);
        // in the middle: the variables in scope are carried
        let r = rw("MATCH (n:A) CALL dbms.components() YIELD name, edition RETURN n.k AS k, name, edition");
        assert_eq!(
            printed(&r),
            "MATCH (n:A) UNWIND $__og_proc1 AS __og_procrow1 WITH n, __og_procrow1.name AS name, __og_procrow1.edition AS edition \
             RETURN n.k AS k, name, edition"
        );
        // a VOID procedure in a statement changes no row
        let r = rw("CALL db.awaitIndexes(300) MATCH (n) RETURN count(n) AS c");
        assert_eq!(printed(&r), "MATCH (n) RETURN count(n) AS c");
        // inside CALL { }, with what the body imports
        let r = rw("MATCH (s) CALL (s) { CALL dbms.components() YIELD name, edition RETURN name AS nm } RETURN s, nm");
        let Some(Clause::Subquery { body, .. }) = r.query.clauses.get(1) else { panic!("{:?}", r.query.clauses) };
        let body = super::super::cypher::Printer::new(None, String::new()).query(body).unwrap();
        assert!(body.contains("WITH s, __og_procrow1.name AS name"), "{body}");
    }

    #[test]
    fn unknown_procedures_and_outputs() {
        let e = rw_p("CALL db.index.vector.queryNodes('v', 2, $v) YIELD node, score RETURN node", json!({})).err().unwrap();
        assert_eq!(e.code, "Neo.ClientError.Procedure.ProcedureNotFound");
        assert!(e.message.starts_with("There is no procedure with the name `db.index.vector.queryNodes` registered") && e.message.contains("SQL"), "{}", e.message);
        let e = rw_p("MATCH (n) CALL nope.nothing() YIELD x RETURN x", json!({})).err().unwrap();
        assert_eq!(e.code, "Neo.ClientError.Procedure.ProcedureNotFound");
        let e = rw_p("CALL db.labels() YIELD nope RETURN nope", json!({})).err().unwrap();
        assert_eq!((e.code.as_str(), e.message.as_str()), ("Neo.ClientError.Statement.SyntaxError", "Unknown procedure output: `nope`"));
        assert!(rewrite(&cypher_syntax::parse("MATCH (n) RETURN n").unwrap(), None).unwrap().is_none());
    }

    #[test]
    fn tohop_is_a_variable_length_match() {
        // STUDIO G5c, Studio's neighbourhood statement
        let q = "MATCH (n:_Entity) WHERE elementId(n) = $node_id CALL apoc.neighbors.tohop(n, '>', $depth) YIELD node AS m \
                 WITH n, m OPTIONAL MATCH (m)-[r]->(o:_Entity) OPTIONAL MATCH (p:_Entity)-[r2]->(m) RETURN m, r, o, r2, p";
        let r = rw_p(q, json!({"node_id": "1", "depth": 2})).ok().unwrap();
        assert!(r.reads.is_empty());
        assert!(
            printed(&r).contains("MATCH (n)-[*1..2]->(m) WHERE id(m) <> id(n) WITH DISTINCT n, m WITH n, m OPTIONAL MATCH (m)-[r]->(o:_Entity)"),
            "{}",
            printed(&r)
        );
        // the filter: types, direction
        let pr = |spec: &str| {
            let r = rw_p(&format!("MATCH (n) CALL apoc.neighbors.tohop(n, '{spec}', 3) YIELD node RETURN node"), json!({})).ok().unwrap();
            printed(&r)
        };
        // several types go out as AGE takes them (cypher.rs lift_alternatives)
        let both = pr("<KNOWS|<LIKES");
        assert!(both.contains("MATCH (n)<-[") && both.contains("*1..3]-(node)") && both.contains("'KNOWS', 'LIKES'"), "{both}");
        assert!(pr("KNOWS").contains("MATCH (n)-[:KNOWS*1..3]-(node)"), "{}", pr("KNOWS"));
        assert!(pr("").contains("MATCH (n)-[*1..3]-(node)"), "{}", pr(""));
        assert!(pr("KNOWS>|>").contains("MATCH (n)-[*1..3]->(node)"), "{}", pr("KNOWS>|>"));
        let e = rw_p("MATCH (n) CALL apoc.neighbors.tohop(n, 'A>|<B', 2) YIELD node RETURN node", json!({})).err().unwrap();
        assert!(e.message.contains("more than one direction"), "{}", e.message);
        // the distance: known before the statement is printed
        let e = rw_p("MATCH (n) CALL apoc.neighbors.tohop(n, '>', n.d) YIELD node RETURN node", json!({})).err().unwrap();
        assert!(e.message.contains("literal or a parameter"), "{}", e.message);
        let e = rw_p("MATCH (n) CALL apoc.neighbors.tohop(n, '>', $d) YIELD node RETURN node", json!({})).err().unwrap();
        assert_eq!(e.code, "Neo.ClientError.Statement.ParameterMissing");
        let e = rw_p("MATCH (n) CALL apoc.neighbors.tohop(n, '>', 0) YIELD node RETURN node", json!({})).err().unwrap();
        assert_eq!(e.code, "Neo.ClientError.Statement.ArgumentError");
        // ending with the call: it returns what it yields
        let r = rw_p("MATCH (n {k: 1}) CALL apoc.neighbors.tohop(n, '>', 1) YIELD node", json!({})).ok().unwrap();
        assert!(printed(&r).ends_with("WITH DISTINCT n, node RETURN node"), "{}", printed(&r));
    }

    #[test]
    fn index_columns_are_read_back() {
        assert_eq!(index_key("agtype_access_operator(VARIADIC ARRAY[properties, '\"id\"'::agtype])").as_deref(), Some("id"));
        assert_eq!(
            index_key("ag_catalog.agtype_access_operator(VARIADIC ARRAY[properties, '\"it''s\"'::ag_catalog.agtype])").as_deref(),
            Some("it's")
        );
        assert_eq!(index_key("id"), None);
        assert_eq!(index_key("agtype_access_operator(VARIADIC ARRAY[properties, '\"a\"'::agtype, '\"b\"'::agtype])"), None);
        let row = |n: &str, l: &str, k: &str, u: &str, def: &str| {
            vec![Some(n.into()), Some(l.into()), Some(k.into()), Some(u.into()), Some("t".into()), Some("16500".into()), Some(def.into())]
        };
        let def = |p: &str| format!("agtype_access_operator(VARIADIC ARRAY[properties, '\"{p}\"'::agtype])");
        let ixs = indexes_from_rows(&[
            row("ont_pub_customer_key", "Customer", "v", "t", &def("customer_id")),
            row("index_Doc_a_b", "Doc", "v", "f", &def("a")),
            row("index_Doc_a_b", "Doc", "v", "f", &def("b")),
            row("someone_elses", "Doc", "v", "f", "lower((properties)::text)"),
            row("index_R_w", "R", "e", "f", &def("w")),
        ]);
        assert_eq!(ixs.len(), 3);
        assert_eq!(ixs[1].props, vec!["a", "b"]);
        let ix = index_rows(&ixs);
        assert_eq!(ix[0]["state"], json!("ONLINE"));
        assert_eq!(ix[0]["owningConstraint"], json!("ont_pub_customer_key"));
        assert_eq!(ix[1]["createStatement"], json!("CREATE RANGE INDEX `index_Doc_a_b` FOR (n:`Doc`) ON (n.`a`, n.`b`)"));
        assert_eq!(ix[2]["entityType"], json!("RELATIONSHIP"));
        assert_eq!(ix[1]["options"], json!({"indexProvider": "range-1.0", "indexConfig": {}}));
        for c in INDEX_COLUMNS {
            assert!(ix[0].contains_key(*c), "{c}");
        }
        let cs = constraint_rows(&ixs);
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0]["labelsOrTypes"], json!(["Customer"]));
        assert_eq!(cs[0]["createStatement"], json!("CREATE CONSTRAINT `ont_pub_customer_key` FOR (n:`Customer`) REQUIRE (n.`customer_id`) IS UNIQUE"));
        for c in CONSTRAINT_COLUMNS {
            assert!(cs[0].contains_key(*c), "{c}");
        }
    }

    /// Answers SQL and Cypher from a script, in order.
    struct Fake {
        asked: Vec<String>,
        sql: Vec<Vec<Vec<Option<String>>>>,
        cypher: Vec<Vec<Vec<Ag>>>,
    }

    impl Exec for Fake {
        fn cypher(&mut self, text: &str, _ncols: usize, _params: &Json) -> Result<Vec<Vec<Ag>>, Refusal> {
            self.asked.push(text.to_string());
            Ok(if self.cypher.is_empty() { Vec::new() } else { self.cypher.remove(0) })
        }
        fn sql(&mut self, sql: &str) -> Result<Vec<Vec<Option<String>>>, Refusal> {
            self.asked.push(sql.to_string());
            Ok(if self.sql.is_empty() { Vec::new() } else { self.sql.remove(0) })
        }
        fn graph(&self) -> String {
            "g".into()
        }
    }

    #[test]
    fn meta_schema_samples_through_cypher() {
        let s = |x: &str| Some(x.to_string());
        let mut f = Fake {
            asked: vec![],
            // the hierarchy: Customer inherits _Entity; an edge type R
            sql: vec![
                vec![vec![s("Customer"), s("v"), s("_Entity")], vec![s("_Entity"), s("v"), None], vec![s("R"), s("e"), None]],
                vec![vec![s("constraint_Customer_k"), s("Customer"), s("v"), s("t"), s("t"), s("1"), s("agtype_access_operator(VARIADIC ARRAY[properties, '\"k\"'::agtype])")]],
            ],
            cypher: vec![
                vec![vec![Ag::Str("Customer".into()), Ag::Int(2)], vec![Ag::Str("_Entity".into()), Ag::Int(1)]],
                vec![vec![Ag::Str("R".into()), Ag::Int(5)]],
                // Customer's sample, then _Entity's
                vec![
                    vec![Ag::Map(vec![("k".into(), Ag::Str("c1".into())), ("n".into(), Ag::Int(1))])],
                    vec![Ag::Map(vec![("k".into(), Ag::Str("c2".into())), ("n".into(), Ag::Float(1.5)), ("tags".into(), Ag::List(vec![]))])],
                ],
                vec![vec![Ag::Map(vec![("name".into(), Ag::Str("e".into()))])]],
                vec![vec![Ag::Map(vec![("w".into(), Ag::Int(3))])]],
            ],
        };
        let rows = rows(&mut f, Source::MetaSchema).unwrap();
        let v = &rows[0]["value"];
        assert_eq!(v["Customer"]["count"], json!(2));
        assert_eq!(v["_Entity"]["count"], json!(3));
        assert_eq!(v["Customer"]["labels"], json!(["_Entity"]));
        assert_eq!(v["Customer"]["properties"]["k"], json!({"type": "STRING", "array": false, "indexed": true, "unique": true, "existence": false}));
        assert_eq!(v["Customer"]["properties"]["tags"]["type"], json!("LIST"));
        // a parent label has its own properties and those of the labels below it
        let ek: Vec<&String> = v["_Entity"]["properties"].as_object().unwrap().keys().collect();
        assert_eq!(ek, vec!["k", "n", "name", "tags"]);
        assert_eq!(v["R"], json!({"type": "relationship", "count": 5, "properties": {"w": {"type": "INTEGER", "array": false, "indexed": false, "unique": false, "existence": false}}}));
        assert!(f.asked.iter().any(|q| q == "MATCH (n:Customer) WHERE label(n) = 'Customer' WITH n LIMIT 100 RETURN properties(n)"), "{:#?}", f.asked);
        assert!(f.asked.iter().any(|q| q == "MATCH ()-[r:R]->() WITH r LIMIT 100 RETURN properties(r)"), "{:#?}", f.asked);
        // no SQL reads data: the catalog only
        assert!(f.asked.iter().filter(|q| q.starts_with("SELECT")).all(|q| q.contains("ag_catalog.ag_label")), "{:#?}", f.asked);
    }

    #[test]
    fn rows_become_the_parameter() {
        let read = Read { source: Source::Indexes, param: "__og_proc1".into(), cols: vec!["name".into(), "state".into()], single: false };
        let rows = vec![json!({"name": "a", "state": "ONLINE", "type": "RANGE"}).as_object().cloned().unwrap()];
        assert_eq!(as_param(&read, &rows), json!([{"name": "a", "state": "ONLINE"}]));
        let read = Read { source: Source::Labels, param: "__og_proc1".into(), cols: vec!["label".into()], single: true };
        assert_eq!(as_param(&read, &one_col("label", ["A".to_string(), "B".to_string()])), json!(["A", "B"]));
        assert_eq!(json_to_ag(&json!({"a": [1, 1.5, null]})), Ag::Map(vec![("a".into(), Ag::List(vec![Ag::Int(1), Ag::Float(1.5), Ag::Null]))]));
        assert_eq!(json_to_ag(&json!(100.0)), Ag::Float(100.0));
        // dbms.components: the server's agent
        assert_eq!(component_rows()[0]["versions"], json!(["4.4.0"]));
    }
}
