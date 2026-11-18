//! The AGE backend: the gateway in front of Apache AGE instead of Ontological
//! (`OG_BOLT_BACKEND=age`).
//!
//! On the default backend the gateway parses nothing and every statement goes
//! to `og_cypher()`. AGE has no such entry point — Cypher reaches it as
//! `SELECT * FROM ag_catalog.cypher('<graph>', $$…$$, $1) AS (c0 agtype, …)`,
//! which needs the column list up front — and no parse-only function, no write
//! counters, no Neo4j schema commands. So here the gateway does those itself:
//!
//! - `cypher.rs` reads the statement with the engine's parser and re-prints it
//!   as AGE's Cypher (the RETURN columns, read/write, the rewrites);
//! - `agtype.rs` turns AGE's values into Bolt's (vertex → Node, edge →
//!   Relationship, path → Path);
//! - `ddl.rs` answers `CREATE CONSTRAINT/INDEX` and `DROP` with expression
//!   indexes on the label's table;
//! - `split.rs` runs what AGE's Cypher has no clause for (`FOREACH`,
//!   `CALL { … }`, `SET n:L` / `REMOVE n:L`) as several statements in one
//!   transaction;
//! - the write counters come from PostgreSQL's per-transaction table
//!   statistics (`pg_stat_xact_user_tables`) of the graph's label tables,
//!   read before and after the statement ([`Counts`]).
//!
//! AGE itself is used as it is; nothing here needs a patched AGE.

pub mod agtype;
pub mod cypher;
pub mod ddl;
pub mod split;

use cypher_syntax::ast::{Clause, Query};
use serde_json::{Map, Value as Json};

/// How the AGE backend is configured.
pub struct AgeConfig {
    /// Make a graph the first time a session uses it (`create_graph`). Neo4j
    /// and Ontological answer "database not found" instead; a project-per-graph
    /// client has no Cypher for making one, so this is on by default.
    pub create_graphs: bool,
    /// `LOAD 'age'` at login, for a server that does not preload AGE
    /// (`shared_preload_libraries`). Only a superuser may; off by default.
    pub load: bool,
    /// Write every SQL statement sent to AGE, and how long it took, to stderr.
    pub trace: bool,
}

fn flag(v: Option<String>, default: bool) -> bool {
    match v.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        Some("0" | "false" | "no" | "off") => false,
        Some("1" | "true" | "yes" | "on") => true,
        _ => default,
    }
}

impl AgeConfig {
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> AgeConfig {
        AgeConfig {
            create_graphs: flag(get("OG_BOLT_AGE_CREATE_GRAPHS"), true),
            load: flag(get("OG_BOLT_AGE_LOAD"), false),
            trace: flag(get("OG_BOLT_AGE_TRACE"), false),
        }
    }
}

/// What a statement becomes.
pub enum Plan {
    /// One `cypher()` call.
    Cypher { text: String, ncols: usize },
    /// A schema command, answered with SQL on the label tables.
    Ddl(cypher_syntax::ast::Ddl),
    /// A procedure the gateway answers itself.
    Procedure(ddl::Procedure),
    /// Several `cypher()` calls and SQL steps in one transaction.
    Steps(split::Steps),
    /// `SHOW GRAPHS` / `CREATE GRAPH name`, the graph statements Ontological
    /// takes over Bolt (Robo's launcher lists and makes projects with them).
    Graphs(GraphStatement),
}

#[derive(Debug, PartialEq)]
pub enum GraphStatement {
    Show,
    Create(String),
}

impl GraphStatement {
    pub fn sql(&self) -> String {
        match self {
            GraphStatement::Show => "SELECT g.name::text AS graph, \
                (SELECT count(*) FROM ag_catalog.ag_label l WHERE l.graph = g.graphid \
                   AND l.name NOT IN ('_ag_label_vertex', '_ag_label_edge'))::int8 AS types \
                FROM ag_catalog.ag_graph g ORDER BY 1"
                .into(),
            GraphStatement::Create(n) => format!("SELECT ag_catalog.create_graph({}::name)", sql_lit(n)),
        }
    }
}

/// The words Ontological's statement layer (`engine/src/fabric/sqlish.rs`)
/// takes before Cypher. Graph statements are answered here; the fabric ones
/// (datasources, virtual nodes, QUERY FABRIC …) belong to the fabric service,
/// not to a graph store, and are refused by name.
fn statement_words(query: &str) -> Vec<String> {
    let mut s = query.trim();
    loop {
        if let Some(rest) = s.strip_prefix("--").or_else(|| s.strip_prefix("//")) {
            s = rest.split_once('\n').map_or("", |(_, r)| r).trim_start();
        } else if let Some(rest) = s.strip_prefix("/*") {
            s = rest.split_once("*/").map_or("", |(_, r)| r).trim_start();
        } else {
            break;
        }
    }
    s.trim_end_matches(|c: char| c == ';' || c.is_whitespace()).split_whitespace().take(4).map(str::to_string).collect()
}

fn graph_or_fabric(query: &str) -> Option<Result<Statement, Refusal>> {
    let w = statement_words(query);
    let up: Vec<String> = w.iter().map(|x| x.to_ascii_uppercase()).collect();
    let up: Vec<&str> = up.iter().map(String::as_str).collect();
    let plan = |g: GraphStatement, columns: &[&str], write: bool| {
        Some(Ok(Statement { plan: Plan::Graphs(g), columns: columns.iter().map(|c| c.to_string()).collect(), write, extra: Map::new() }))
    };
    match up.as_slice() {
        ["SHOW", "GRAPHS", ..] if up.len() == 2 => plan(GraphStatement::Show, &["graph", "types"], false),
        ["CREATE", "GRAPH", _] => plan(GraphStatement::Create(w[2].clone()), &["result"], true),
        ["CREATE", "DATASOURCE" | "VIRTUAL", ..]
        | ["DROP", "DATASOURCE" | "VIRTUAL", ..]
        | ["ALTER", "DATASOURCE", ..]
        | ["SHOW", "DATASOURCES" | "CREDENTIALS" | "TABLES" | "COLUMNS" | "COLUMN" | "VALUE" | "VIRTUAL", ..]
        | ["REFRESH", "DATASOURCE", ..]
        | ["MATERIALIZE", ..]
        | ["EXPLAIN", "FABRIC", ..]
        | ["QUERY", "FABRIC", ..]
        | ["USE", "GRAPH", ..] => Some(Err(Refusal {
            code: "Neo.ClientError.Statement.SyntaxError".into(),
            message: format!(
                "{} is an Ontological fabric statement; the AGE backend stores graphs only — the fabric is its own service",
                up.iter().take(2).cloned().collect::<Vec<_>>().join(" ")
            ),
        })),
        _ => None,
    }
}

pub struct Statement {
    pub plan: Plan,
    /// The RETURN columns, in order (Bolt's `fields`).
    pub columns: Vec<String>,
    pub write: bool,
    /// Parameters the rewrites added, merged into the statement's own.
    pub extra: Map<String, Json>,
}

/// A failure, as the code a Neo4j driver expects and a message.
#[derive(Debug)]
pub struct Refusal {
    pub code: String,
    pub message: String,
}

impl Refusal {
    pub fn syntax(message: String) -> Refusal {
        Refusal { code: "Neo.ClientError.Statement.SyntaxError".into(), message }
    }
}

/// Parse only: does it write? A query that does not parse gets its syntax
/// error now, as `og_cypher_check()` gives it on the default backend.
pub fn check(query: &str) -> Result<bool, Refusal> {
    if let Some(st) = graph_or_fabric(query) {
        return st.map(|s| s.write);
    }
    let q = cypher_syntax::parse(query).map_err(Refusal::syntax)?;
    Ok(cypher::is_write(&q))
}

/// Read and re-print one statement.
pub fn translate(query: &str, params: &Json) -> Result<Statement, Refusal> {
    if let Some(st) = graph_or_fabric(query) {
        return st;
    }
    let q: Query = cypher_syntax::parse(query).map_err(Refusal::syntax)?;
    // The fields and read/write are the statement's as sent.
    let write = cypher::is_write(&q);
    let columns = cypher::columns(&q);
    // Not cut (split.rs) inside CALL { … }; folded there instead.
    let q = cypher::fold_nested_merge_sets(&q);
    let q = cypher::lead_optional(&cypher::split_aggregations(&cypher::rename_clashing(&q)));
    if let Some(Clause::Ddl(d)) = q.clauses.first() {
        if q.clauses.len() > 1 {
            return Err(Refusal::syntax("a schema command cannot be combined with other clauses".into()));
        }
        return Ok(Statement { plan: Plan::Ddl(d.clone()), columns, write: true, extra: Map::new() });
    }
    if let Some(p) = ddl::Procedure::recognise(&q) {
        let columns = p.columns();
        return Ok(Statement { plan: Plan::Procedure(p), columns, write: false, extra: Map::new() });
    }
    let pmap = params.as_object();
    let mut printer = cypher::Printer::new(pmap, cypher::now_text());
    if split::needed(&q) {
        let steps = split::plan(&q, &mut printer).map_err(Refusal::syntax)?;
        return Ok(Statement { plan: Plan::Steps(steps), columns, write, extra: printer.extra });
    }
    let text = printer.query(&q).map_err(Refusal::syntax)?;
    Ok(Statement { plan: Plan::Cypher { text, ncols: columns.len() }, columns, write, extra: printer.extra })
}

/// A SQL string literal.
pub fn sql_lit(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// A SQL identifier, always quoted.
pub fn sql_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// The SQL that runs one AGE Cypher text with `$1` as its parameter map.
///
/// The text goes in dollar-quoted, with a tag it does not contain; AGE takes
/// only a dollar-quoted constant there. Parameter values never enter the text.
pub fn cypher_sql(graph: &str, text: &str, ncols: usize) -> String {
    let mut tag = String::from("$og$");
    let mut n = 0;
    while text.contains(&tag) {
        n += 1;
        tag = format!("$og{n}$");
    }
    // A statement without RETURN still needs one column in the definition.
    let cols = (0..ncols.max(1)).map(|i| format!("c{i} ag_catalog.agtype")).collect::<Vec<_>>().join(", ");
    format!("SELECT * FROM ag_catalog.cypher({}, {tag}{text}{tag}, $1) AS ({cols})", sql_lit(graph))
}

/// Is there a graph by this name?
pub const GRAPH_EXISTS: &str = "SELECT 1 FROM ag_catalog.ag_graph WHERE name = $1::text::name";
pub const CREATE_GRAPH: &str = "SELECT ag_catalog.create_graph($1::text::name)";

/// Inserted, deleted and updated rows of the graph's vertex and edge label
/// tables, so far in this transaction. `pg_stat_xact_user_tables` counts what
/// the heap did, and AGE's executor writes through the heap, so the difference
/// before and after a statement is what it changed: one row per vertex or
/// edge created or deleted, one per element updated (however many of its
/// properties changed — the one approximation).
pub const COUNTS: &str = "SELECT \
    coalesce(sum(pg_stat_get_xact_tuples_inserted(l.relation)) FILTER (WHERE l.kind = 'v'), 0)::int8, \
    coalesce(sum(pg_stat_get_xact_tuples_deleted(l.relation)) FILTER (WHERE l.kind = 'v'), 0)::int8, \
    coalesce(sum(pg_stat_get_xact_tuples_updated(l.relation)) FILTER (WHERE l.kind = 'v'), 0)::int8, \
    coalesce(sum(pg_stat_get_xact_tuples_inserted(l.relation)) FILTER (WHERE l.kind = 'e'), 0)::int8, \
    coalesce(sum(pg_stat_get_xact_tuples_deleted(l.relation)) FILTER (WHERE l.kind = 'e'), 0)::int8, \
    coalesce(sum(pg_stat_get_xact_tuples_updated(l.relation)) FILTER (WHERE l.kind = 'e'), 0)::int8 \
    FROM ag_catalog.ag_label l JOIN ag_catalog.ag_graph g ON g.graphid = l.graph WHERE g.name = $1::text::name";

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Counts {
    pub v_ins: i64,
    pub v_del: i64,
    pub v_upd: i64,
    pub e_ins: i64,
    pub e_del: i64,
    pub e_upd: i64,
}

impl Counts {
    pub fn from_row(row: &postgres::Row) -> Counts {
        Counts {
            v_ins: row.get(0),
            v_del: row.get(1),
            v_upd: row.get(2),
            e_ins: row.get(3),
            e_del: row.get(4),
            e_upd: row.get(5),
        }
    }

    pub fn minus(self, before: Counts) -> Counts {
        Counts {
            v_ins: self.v_ins - before.v_ins,
            v_del: self.v_del - before.v_del,
            v_upd: self.v_upd - before.v_upd,
            e_ins: self.e_ins - before.e_ins,
            e_del: self.e_del - before.e_del,
            e_upd: self.e_upd - before.e_upd,
        }
    }
}

/// What a write changed, in Neo4j's spelling — the keys `og_cypher_stats()`
/// gives on the default backend.
#[derive(Default, Debug, Clone, PartialEq)]
pub struct Stats {
    pub nodes_created: i64,
    pub nodes_deleted: i64,
    pub relationships_created: i64,
    pub relationships_deleted: i64,
    pub properties_set: i64,
    pub labels_added: i64,
    pub labels_removed: i64,
    pub indexes_added: i64,
    pub indexes_removed: i64,
    pub constraints_added: i64,
    pub constraints_removed: i64,
}

impl Stats {
    /// From the row counts. A relabelled vertex (`split.rs`) is a deleted row
    /// and an inserted one; the steps that do it subtract themselves again.
    pub fn from_counts(c: Counts) -> Stats {
        Stats {
            nodes_created: c.v_ins.max(0),
            nodes_deleted: c.v_del.max(0),
            relationships_created: c.e_ins.max(0),
            relationships_deleted: c.e_del.max(0),
            // An element updated is at least one property set; a new one's
            // properties are set as it is made.
            properties_set: (c.v_upd + c.e_upd).max(0),
            // Every vertex AGE makes carries its one label.
            labels_added: c.v_ins.max(0),
            ..Stats::default()
        }
    }

    pub fn add(&mut self, o: &Stats) {
        self.nodes_created += o.nodes_created;
        self.nodes_deleted += o.nodes_deleted;
        self.relationships_created += o.relationships_created;
        self.relationships_deleted += o.relationships_deleted;
        self.properties_set += o.properties_set;
        self.labels_added += o.labels_added;
        self.labels_removed += o.labels_removed;
        self.indexes_added += o.indexes_added;
        self.indexes_removed += o.indexes_removed;
        self.constraints_added += o.constraints_added;
        self.constraints_removed += o.constraints_removed;
    }

    /// As the Bolt summary's `stats` map.
    pub fn to_json(&self) -> Json {
        let pairs = [
            ("nodes-created", self.nodes_created),
            ("nodes-deleted", self.nodes_deleted),
            ("relationships-created", self.relationships_created),
            ("relationships-deleted", self.relationships_deleted),
            ("properties-set", self.properties_set),
            ("labels-added", self.labels_added),
            ("labels-removed", self.labels_removed),
            ("indexes-added", self.indexes_added),
            ("indexes-removed", self.indexes_removed),
            ("constraints-added", self.constraints_added),
            ("constraints-removed", self.constraints_removed),
        ];
        let total: i64 = pairs.iter().map(|(_, n)| *n).sum();
        let mut m = Map::new();
        for (k, n) in pairs {
            m.insert(k.into(), Json::from(n));
        }
        m.insert("contains-updates".into(), Json::Bool(total > 0));
        Json::Object(m)
    }
}

/// The Neo4j status code for an error from AGE.
///
/// AGE reports through PostgreSQL's SQLSTATEs, so a few that mean something
/// to a Cypher client are read here first; the rest go through the gateway's
/// usual table (`session::error_code`).
pub fn error_code(state: Option<&str>, message: &str) -> Option<&'static str> {
    Some(match state? {
        // invalid_schema_name: `graph "x" does not exist`
        "3F000" => "Neo.ClientError.Database.DatabaseNotFound",
        "42601" => "Neo.ClientError.Statement.SyntaxError",
        "23505" | "23502" | "23514" => "Neo.ClientError.Schema.ConstraintValidationFailed",
        "0A000" => "Neo.ClientError.Statement.SyntaxError",
        "22023" if message.contains("parameter") => "Neo.ClientError.Statement.ParameterMissing",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sql_quotes_what_it_must() {
        assert_eq!(
            cypher_sql("robo", "MATCH (n) RETURN n, 1", 2),
            "SELECT * FROM ag_catalog.cypher('robo', $og$MATCH (n) RETURN n, 1$og$, $1) AS (c0 ag_catalog.agtype, c1 ag_catalog.agtype)"
        );
        // A text holding the tag gets another one; a graph name with a quote is doubled.
        let s = cypher_sql("o'k", "RETURN '$og$' AS x", 1);
        assert!(s.starts_with("SELECT * FROM ag_catalog.cypher('o''k', $og1$RETURN '$og$' AS x$og1$, $1)"), "{s}");
        // No RETURN: one column all the same.
        assert!(cypher_sql("g", "CREATE (n)", 0).ends_with("AS (c0 ag_catalog.agtype)"));
    }

    #[test]
    fn graph_statements_and_fabric_ones() {
        let st = translate("SHOW GRAPHS;", &Json::Null).ok().unwrap();
        assert_eq!((st.columns.clone(), st.write), (vec!["graph".to_string(), "types".to_string()], false));
        let st = translate("// new project\nCREATE GRAPH shop", &Json::Null).ok().unwrap();
        assert!(matches!(st.plan, Plan::Graphs(GraphStatement::Create(ref n)) if n == "shop"));
        let e = translate("QUERY FABRIC SELECT 1", &Json::Null).err().unwrap();
        assert!(e.message.contains("fabric"), "{}", e.message);
        assert!(check("SHOW GRAPHS").is_ok_and(|w| !w));
    }

    #[test]
    fn counters_come_from_row_counts() {
        let before = Counts { v_ins: 2, ..Counts::default() };
        let after = Counts { v_ins: 5, v_del: 1, e_ins: 2, e_del: 3, v_upd: 4, e_upd: 0 };
        let s = Stats::from_counts(after.minus(before));
        assert_eq!((s.nodes_created, s.nodes_deleted, s.relationships_created, s.relationships_deleted), (3, 1, 2, 3));
        assert_eq!((s.properties_set, s.labels_added), (4, 3));
        let j = s.to_json();
        assert_eq!(j["nodes-deleted"], Json::from(1));
        assert_eq!(j["contains-updates"], Json::Bool(true));
        assert_eq!(Stats::default().to_json()["contains-updates"], Json::Bool(false));
    }

    #[test]
    fn statements_are_classified_by_the_engines_parser() {
        assert!(!check("MATCH (n) RETURN n // CREATE (x)").unwrap());
        assert!(check("MATCH (n) SET n.x = 'CREATE'").unwrap());
        assert_eq!(check("MATCH (n RETURN n").err().unwrap().code, "Neo.ClientError.Statement.SyntaxError");
        let st = translate("CREATE CONSTRAINT c IF NOT EXISTS FOR (n:L) REQUIRE n.id IS UNIQUE", &Json::Null).ok().unwrap();
        assert!(matches!(st.plan, Plan::Ddl(_)) && st.write);
        let st = translate("MATCH (n:L) RETURN n.id AS id, count(*)", &Json::Null).ok().unwrap();
        assert_eq!(st.columns, vec!["id", "count(*)"]);
        let Plan::Cypher { ncols, .. } = st.plan else { panic!() };
        assert_eq!(ncols, 2);
    }
}
