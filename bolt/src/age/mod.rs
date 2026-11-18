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
//! - `reach.rs` walks a long variable-length relationship AGE cannot (more
//!   than three hops without exactly one type) breadth first in SQL, where only
//!   the nodes it reaches matter;
//! - `hierarchy.rs` gives a node several labels the way AGE can hold it: one
//!   table, which inherits the tables of its other labels (PostgreSQL
//!   inheritance, made before a `CREATE`/`MERGE (n:A:B)` runs); the reads of
//!   labels are printed with that hierarchy, read from the catalog for each
//!   statement that needs it;
//! - the write counters come from PostgreSQL's per-transaction table
//!   statistics (`pg_stat_xact_user_tables`) of the graph's label tables,
//!   read before and after the statement ([`Counts`]).
//!
//! AGE itself is used as it is; nothing here needs a patched AGE.

pub mod agtype;
pub mod cypher;
pub mod ddl;
pub mod hierarchy;
pub mod reach;
pub mod split;

use cypher_syntax::ast::{Clause, Query};
use hierarchy::{Hierarchy, Schema};
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
    /// Read the write counters (two short SQL statements around every write).
    /// On by default; a client that never reads `summary.counters` may turn
    /// them off.
    pub counters: bool,
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
            counters: flag(get("OG_BOLT_AGE_COUNTERS"), true),
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
    /// A long variable-length relationship walked by the gateway (`reach.rs`):
    /// the seeds, a breadth-first walk in SQL, the statement.
    Reach(reach::Reach),
    /// `SHOW GRAPHS` / `CREATE GRAPH name`, the graph statements Ontological
    /// takes over Bolt (Robo's launcher lists and makes projects with them).
    Graphs(GraphStatement),
}

impl Plan {
    /// What the trace calls it.
    pub fn kind(&self) -> &'static str {
        match self {
            Plan::Cypher { .. } => "cypher",
            Plan::Ddl(_) => "ddl",
            Plan::Procedure(_) => "procedure",
            Plan::Steps(_) => "steps",
            Plan::Reach(_) => "reach",
            Plan::Graphs(_) => "graphs",
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum GraphStatement {
    Show,
    Create(String),
}

impl GraphStatement {
    /// The SQL: graph names as AGE has them ([`age_graph_name`]); the rows
    /// of `SHOW GRAPHS` are mapped back by [`bolt_database_name`].
    pub fn sql(&self) -> String {
        match self {
            GraphStatement::Show => "SELECT g.name::text AS graph, \
                (SELECT count(*) FROM ag_catalog.ag_label l WHERE l.graph = g.graphid \
                   AND l.name NOT IN ('_ag_label_vertex', '_ag_label_edge'))::int8 AS types \
                FROM ag_catalog.ag_graph g ORDER BY 1"
                .into(),
            GraphStatement::Create(n) => format!("SELECT ag_catalog.create_graph({}::name)", sql_lit(&age_graph_name(n))),
        }
    }
}

// ------------------------------------------------------------ graph names

/// What a Bolt database name of fewer than 3 characters is stored under.
///
/// AGE refuses a graph name shorter than 3 characters (`name_validation.h`:
/// `MIN_GRAPH_NAME_LEN 3`, "graph name is invalid"; SUITES N10/A14), and
/// Robo's launcher makes projects of 2 (`projects.ts`: `^[a-z][a-z0-9_]{1,39}$`).
/// Such a name `ab` is the AGE graph `_og_ab`. The image cannot be a name a
/// client means for itself: Neo4j's database names start with a letter, and
/// the gateway refuses a database name of that exact form (`_og_` and one or
/// two characters, [`reserved_database_name`]); every name of 3 characters
/// or more is the graph of the same name, as before. Only `[A-Za-z0-9_]` is
/// used, the characters AGE and the gateway's SQL take unquoted.
pub const SHORT_GRAPH_PREFIX: &str = "_og_";

/// The shortest graph name AGE takes.
pub const MIN_GRAPH_NAME: usize = 3;

/// The AGE graph a Bolt database name is ([`SHORT_GRAPH_PREFIX`]).
pub fn age_graph_name(db: &str) -> String {
    if db.chars().count() < MIN_GRAPH_NAME {
        format!("{SHORT_GRAPH_PREFIX}{db}")
    } else {
        db.to_string()
    }
}

/// The Bolt database name an AGE graph is shown as (`SHOW GRAPHS`): the
/// inverse of [`age_graph_name`].
pub fn bolt_database_name(graph: &str) -> String {
    match graph.strip_prefix(SHORT_GRAPH_PREFIX) {
        Some(short) if !short.is_empty() && short.chars().count() < MIN_GRAPH_NAME => short.to_string(),
        _ => graph.to_string(),
    }
}

/// Is this the AGE name of a short database name — refused as a database
/// name, so that two names never meet in one graph?
pub fn reserved_database_name(db: &str) -> bool {
    bolt_database_name(db) != db
}

/// Can a session name this database? `Err`: why not.
pub fn check_database_name(db: &str) -> Result<(), String> {
    let ok = db.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && db.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !ok {
        return Err(format!("'{db}' is not a graph name AGE takes (letters, digits and _, not starting with a digit)"));
    }
    if reserved_database_name(db) {
        return Err(format!(
            "'{db}' is reserved: it is the graph the AGE backend keeps database '{}' in (AGE takes no graph name under {MIN_GRAPH_NAME} characters)",
            bolt_database_name(db)
        ));
    }
    if age_graph_name(db).len() > 63 {
        return Err(format!("'{db}' is longer than the 63 characters a graph name may have"));
    }
    Ok(())
}

/// Does the statement need the session's graph to exist (made or found)?
/// `SHOW GRAPHS` and `CREATE GRAPH` are about the graphs, not in one: they
/// run whatever database the session names, and with
/// `OG_BOLT_AGE_CREATE_GRAPHS=0` a session on a database that is not there
/// yet can still make it.
pub fn needs_session_graph(plan: &Plan) -> bool {
    !matches!(plan, Plan::Graphs(_))
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
        Some(Ok(Statement {
            plan: Plan::Graphs(g),
            columns: columns.iter().map(|c| c.to_string()).collect(),
            write,
            extra: Map::new(),
            merge_nulls: Vec::new(),
            schema: Schema::default(),
        }))
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
    /// MERGE properties sent guarded against null (`cypher::guard_merge_nulls`):
    /// a division by zero from this statement is Neo4j's null-merge error.
    pub merge_nulls: Vec<cypher::NullMerge>,
    /// Label tables and inheritance to make first, in the statement's
    /// transaction (a `CREATE`/`MERGE (n:A:B)`; `hierarchy.rs`).
    pub schema: Schema,
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

/// A statement read, before it is translated.
pub enum Parsed {
    /// Answered without the Cypher parser (graph and fabric statements), or
    /// a syntax error.
    Done(Result<Statement, Refusal>),
    Cypher(Query),
}

impl Parsed {
    /// Does it need the graph's label hierarchy ([`hierarchy::needs`])?
    pub fn needs_hierarchy(&self) -> bool {
        matches!(self, Parsed::Cypher(q) if hierarchy::needs(q))
    }
}

pub fn parse(query: &str) -> Parsed {
    if let Some(st) = graph_or_fabric(query) {
        return Parsed::Done(st);
    }
    match cypher_syntax::parse(query) {
        Ok(q) => Parsed::Cypher(q),
        Err(e) => Parsed::Done(Err(Refusal::syntax(e))),
    }
}

/// Read and re-print one statement, given the graph's label hierarchy (an
/// empty one, when the statement does not need it). The session parses first
/// ([`parse`]) to know whether to read the hierarchy, then
/// [`translate_parsed`]; this is the two in one.
#[allow(dead_code)]
pub fn translate(query: &str, params: &Json, hier: &Hierarchy) -> Result<Statement, Refusal> {
    translate_parsed(parse(query), params, hier)
}

pub fn translate_parsed(parsed: Parsed, params: &Json, hier: &Hierarchy) -> Result<Statement, Refusal> {
    let q: Query = match parsed {
        Parsed::Done(st) => return st,
        Parsed::Cypher(q) => q,
    };
    // The fields and read/write are the statement's as sent.
    let write = cypher::is_write(&q);
    let columns = cypher::columns(&q);
    // Not cut (split.rs) inside CALL { … }; folded there instead.
    let q = cypher::fold_nested_merge_sets(&q);
    let q = cypher::lead_optional(&cypher::split_aggregations(&cypher::rename_clashing(&q)));
    let plain = |plan: Plan, columns: Vec<String>, write: bool| Statement {
        plan,
        columns,
        write,
        extra: Map::new(),
        merge_nulls: Vec::new(),
        schema: Schema::default(),
    };
    if let Some(Clause::Ddl(d)) = q.clauses.first() {
        if q.clauses.len() > 1 {
            return Err(Refusal::syntax("a schema command cannot be combined with other clauses".into()));
        }
        return Ok(plain(Plan::Ddl(d.clone()), columns, true));
    }
    if let Some(p) = ddl::Procedure::recognise(&q) {
        let columns = p.columns();
        return Ok(plain(Plan::Procedure(p), columns, false));
    }
    let pmap = params.as_object();
    // MERGE keys that are null: refused now, or guarded (N7).
    let (q, merge_nulls) = cypher::guard_merge_nulls(&q, pmap)
        .map_err(|n| Refusal { code: "Neo.ClientError.Statement.SemanticError".into(), message: n.message() })?;
    // A node written with several labels: stored under one, whose table
    // inherits the others' (G1). What is printed reads the hierarchy as it
    // will be once that schema has run.
    let mut hier = hier.clone();
    let (q, schema) = hierarchy::store_all(&q, &mut hier)?;
    let mut printer = cypher::Printer::new(pmap, cypher::now_text()).with_hierarchy(hier);
    if reach::needed(&q) {
        // (the walk runs the statement as it is: the cuts for AGE's wrong
        // answers in one statement are not made there)
        if split::must_cut(&q) {
            return Err(Refusal::syntax(cypher::unsupported(
                "a long variable-length relationship together with FOREACH, CALL { … } or a label change",
            )));
        }
        let r = reach::plan(&q, &mut printer, columns.len()).map_err(Refusal::syntax)?;
        return Ok(Statement { plan: Plan::Reach(r), columns, write, extra: printer.extra, merge_nulls, schema });
    }
    if split::needed(&q, &printer.hier) {
        let steps = split::plan(&q, &mut printer).map_err(Refusal::syntax)?;
        return Ok(Statement { plan: Plan::Steps(steps), columns, write, extra: printer.extra, merge_nulls, schema });
    }
    let text = printer.query(&q).map_err(Refusal::syntax)?;
    Ok(Statement { plan: Plan::Cypher { text, ncols: columns.len() }, columns, write, extra: printer.extra, merge_nulls, schema })
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

/// A failure of a statement whose MERGE properties went guarded against null
/// ([`cypher::guard_merge_nulls`]): AGE's "division by zero" is the guard's,
/// and Neo4j's answer is its null-merge error. With more than one guarded
/// property the gateway cannot tell which one was null, and names them all.
pub fn null_merge_refusal(code: &str, message: &str, guards: &[cypher::NullMerge]) -> Option<Refusal> {
    if guards.is_empty() || code != "Neo.ClientError.Statement.ArithmeticError" || !message.contains("division by zero") {
        return None;
    }
    let mut m = guards[0].message();
    if guards.len() > 1 {
        let others: Vec<String> = guards[1..].iter().map(|g| format!("'{}'", g.key)).collect();
        m.push_str(&format!(" (or of {} — one of the MERGE properties computed in this statement was null)", others.join(", ")));
    }
    Some(Refusal { code: "Neo.ClientError.Statement.SemanticError".into(), message: m })
}

/// The Neo4j status code for an error from AGE.
///
/// AGE reports through PostgreSQL's SQLSTATEs, so a few that mean something
/// to a Cypher client are read here first; the rest go through the gateway's
/// usual table (`session::error_code`).
///
/// Type errors (SUITES B10, B12, G14), where Neo4j answers
/// `Neo.ClientError.Statement.TypeError`:
/// - 22023 "Invalid input parameter types for agtype_mul" (`agtype_ops.c`,
///   for `+ - * / % ^` and unary minus over values they do not take:
///   `sum(m.name * 2)`). The word "parameter" there is the C function's
///   argument, and the gateway used to answer it as `ParameterMissing`;
/// - 0A000 "age SET clause can only update vertex and edges", "DELETE clause
///   can only delete vertices and edges" (`cypher_set.c`, `cypher_delete.c`:
///   a SET or DELETE of a map or a scalar) — not a syntax error;
/// - XX000 "container is not an agtype array" (`agtype_util.c`, N8: `WITH {a:
///   1} AS n SET n.x = 1`), AGE's internal message for the same mistake.
///
/// A missing parameter in AGE is 42P02 ("parameters argument is missing from
/// cypher() function call", `cypher_expr.c`) — `ParameterMissing`. The
/// gateway always passes the map, so it is not expected; a key missing from
/// the map reads as null in AGE.
pub fn error_code(state: Option<&str>, message: &str) -> Option<&'static str> {
    const TYPE: &str = "Neo.ClientError.Statement.TypeError";
    Some(match state? {
        // invalid_schema_name: `graph "x" does not exist` — and AGE's
        // `graph "x" already exists` from create_graph() (graph_commands.c
        // gives that the same code), which is Neo4j's CREATE DATABASE answer
        "3F000" if message.contains("already exists") => "Neo.ClientError.Database.ExistingDatabaseFound",
        "3F000" => "Neo.ClientError.Database.DatabaseNotFound",
        "42601" => "Neo.ClientError.Statement.SyntaxError",
        "23505" | "23502" | "23514" => "Neo.ClientError.Schema.ConstraintValidationFailed",
        "0A000" if message.contains("clause can only update") || message.contains("clause can only delete") => TYPE,
        "0A000" => "Neo.ClientError.Statement.SyntaxError",
        "22023" if message.starts_with("Invalid input parameter type") => TYPE,
        "XX000" if message.contains("container is not an agtype array") => TYPE,
        "42P02" => "Neo.ClientError.Statement.ParameterMissing",
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
        let st = translate("SHOW GRAPHS;", &Json::Null, &Hierarchy::default()).ok().unwrap();
        assert_eq!((st.columns.clone(), st.write), (vec!["graph".to_string(), "types".to_string()], false));
        let st = translate("// new project\nCREATE GRAPH shop", &Json::Null, &Hierarchy::default()).ok().unwrap();
        assert!(matches!(st.plan, Plan::Graphs(GraphStatement::Create(ref n)) if n == "shop"));
        let e = translate("QUERY FABRIC SELECT 1", &Json::Null, &Hierarchy::default()).err().unwrap();
        assert!(e.message.contains("fabric"), "{}", e.message);
        assert!(check("SHOW GRAPHS").is_ok_and(|w| !w));
        // with OG_BOLT_AGE_CREATE_GRAPHS=0 these run whatever database the session names
        assert!(!needs_session_graph(&st.plan));
        let st = translate("MATCH (n) RETURN n", &Json::Null, &Hierarchy::default()).ok().unwrap();
        assert!(needs_session_graph(&st.plan));
    }

    #[test]
    fn a_short_database_name_has_a_graph_no_client_can_name() {
        // SUITES A14 / N10: AGE takes no graph name under 3 characters; Robo's launcher makes 2
        assert_eq!(age_graph_name("pr"), "_og_pr");
        assert_eq!(age_graph_name("a"), "_og_a");
        assert_eq!(bolt_database_name("_og_pr"), "pr");
        assert_eq!(bolt_database_name("_og_a"), "a");
        // 3 characters and more: the graph of that name, as before
        for n in ["robo", "abc", "analyzer_run", "_og_abc", "_og_", "_ogab"] {
            assert_eq!(age_graph_name(n), n);
            assert_eq!(bolt_database_name(n), n);
        }
        // the images are refused as database names; nothing else is
        assert!(check_database_name("_og_pr").is_err_and(|e| e.contains("reserved") && e.contains("'pr'")));
        assert!(check_database_name("_og_a").is_err());
        for ok in ["pr", "a", "robo", "_og_abc", "_private", "x_1"] {
            assert!(check_database_name(ok).is_ok(), "{ok}");
        }
        assert!(check_database_name("1ab").is_err() && check_database_name("my-db").is_err() && check_database_name("").is_err());
        assert!(check_database_name(&"a".repeat(64)).is_err() && check_database_name(&"a".repeat(63)).is_ok());
        // CREATE GRAPH makes the mapped one
        assert_eq!(GraphStatement::Create("pr".into()).sql(), "SELECT ag_catalog.create_graph('_og_pr'::name)");
        assert_eq!(GraphStatement::Create("shop".into()).sql(), "SELECT ag_catalog.create_graph('shop'::name)");
        // a graph that is there already: Neo4j's CREATE DATABASE answer
        assert_eq!(error_code(Some("3F000"), "graph \"shop\" already exists"), Some("Neo.ClientError.Database.ExistingDatabaseFound"));
        assert_eq!(error_code(Some("3F000"), "graph \"shop\" does not exist"), Some("Neo.ClientError.Database.DatabaseNotFound"));
    }

    #[test]
    fn type_errors_are_type_errors() {
        // B10: an operator over a value it does not take is not a missing parameter
        assert_eq!(error_code(Some("22023"), "Invalid input parameter types for agtype_mul"), Some("Neo.ClientError.Statement.TypeError"));
        assert_eq!(error_code(Some("22023"), "Invalid input parameter type for agtype_neg"), Some("Neo.ClientError.Statement.TypeError"));
        // B12 / N8: SET or DELETE of a map or a scalar
        assert_eq!(error_code(Some("XX000"), "container is not an agtype array"), Some("Neo.ClientError.Statement.TypeError"));
        assert_eq!(error_code(Some("0A000"), "age SET clause can only update vertex and edges"), Some("Neo.ClientError.Statement.TypeError"));
        assert_eq!(error_code(Some("0A000"), "DELETE clause can only delete vertices and edges"), Some("Neo.ClientError.Statement.TypeError"));
        assert_eq!(error_code(Some("0A000"), "cypher DELETE clause cannot be rescanned"), Some("Neo.ClientError.Statement.SyntaxError"));
        // a missing parameter map is still that; other 22023s go through the usual table
        assert_eq!(
            error_code(Some("42P02"), "parameters argument is missing from cypher() function call"),
            Some("Neo.ClientError.Statement.ParameterMissing")
        );
        assert_eq!(error_code(Some("22023"), "right() length parameter cannot be null"), None);
        assert_eq!(error_code(Some("XX000"), "something else"), None);
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
    fn a_guarded_merge_key_that_was_null_is_neo4js_error() {
        let g = |k: &str| cypher::NullMerge { kind: "node", key: k.into(), shape: format!("(:W {{{k}: null}})") };
        let r = null_merge_refusal("Neo.ClientError.Statement.ArithmeticError", "division by zero", &[g("key")]).unwrap();
        assert_eq!(r.code, "Neo.ClientError.Statement.SemanticError");
        assert_eq!(r.message, "Cannot merge the following node because of null property value for 'key': (:W {key: null})");
        let r = null_merge_refusal("Neo.ClientError.Statement.ArithmeticError", "division by zero", &[g("a"), g("b")]).unwrap();
        assert!(r.message.contains("for 'a'") && r.message.contains("'b'"), "{}", r.message);
        // nothing guarded, or another failure: as it came
        assert!(null_merge_refusal("Neo.ClientError.Statement.ArithmeticError", "division by zero", &[]).is_none());
        assert!(null_merge_refusal("Neo.ClientError.Statement.TypeError", "division by zero", &[g("a")]).is_none());
        // through translate: a parameter known to be null is refused before anything runs
        let e = translate("MERGE (n:W {key: $k}) RETURN n", &serde_json::json!({"k": null}), &Hierarchy::default()).err().unwrap();
        assert_eq!(e.code, "Neo.ClientError.Statement.SemanticError");
        let st = translate("UNWIND $rows AS r MERGE (n:W {key: r.k}) RETURN n", &serde_json::json!({"rows": []}), &Hierarchy::default()).ok().unwrap();
        assert_eq!(st.merge_nulls.len(), 1);
    }

    #[test]
    fn a_node_with_several_labels_is_stored_under_the_last() {
        let text = |st: &Statement| match &st.plan {
            Plan::Cypher { text, .. } => text.clone(),
            _ => panic!("not one cypher() call"),
        };
        // ANALYZER §5.1, on a graph that has neither label yet
        let st = translate("MERGE (n:ANALYZER_OWNED:TABLE {id: 'x9', graph_owner: 'analyzer'}) RETURN n", &Json::Null, &Hierarchy::of(&[], &[]))
            .ok()
            .unwrap();
        assert_eq!(text(&st), "MERGE (n:TABLE {id: 'x9', graph_owner: 'analyzer'}) RETURN n");
        assert_eq!(st.schema.create, vec!["ANALYZER_OWNED".to_string(), "TABLE".to_string()]);
        assert_eq!(st.schema.inherit, vec![("TABLE".to_string(), "ANALYZER_OWNED".to_string())]);
        assert!(st.schema.empty.is_empty());
        // STUDIO G1, once Customer's table inherits _Entity's: nothing to make
        let h = Hierarchy::of(&[("Customer", "_Entity")], &[]);
        let st = translate("CREATE (n:_Entity:Customer {customer_id: 'x'}) RETURN elementId(n) AS eid", &Json::Null, &h).ok().unwrap();
        assert_eq!(text(&st), "CREATE (n:Customer {customer_id: 'x'}) RETURN toString(id(coalesce(n))) AS eid");
        assert!(st.schema.is_empty());
        // what the statement prints reads the hierarchy its schema makes
        let st = translate("CREATE (c:_Entity:Supplier {k: 1}) RETURN labels(c) AS l", &Json::Null, &h).ok().unwrap();
        assert_eq!(st.schema.inherit, vec![("Supplier".to_string(), "_Entity".to_string())]);
        assert_eq!(
            text(&st),
            "CREATE (c:Supplier {k: 1}) RETURN (CASE label(c) WHEN 'Customer' THEN ['Customer', '_Entity'] \
             WHEN 'Supplier' THEN ['Supplier', '_Entity'] ELSE labels(c) END) AS l"
        );
        // Neo4j's null-merge message names every label
        let e = translate("MERGE (n:_Entity:Customer {k: $k})", &serde_json::json!({"k": null}), &h).err().unwrap();
        assert!(e.message.ends_with("(:_Entity:Customer {k: null})"), "{}", e.message);
        // still refused: a choice of labels, two relationship types
        assert!(translate("MERGE (n:A|B {k: 1})", &Json::Null, &h).err().unwrap().message.contains("not a choice"));
        assert!(translate("MATCH (a), (b) CREATE (a)-[:R|S]->(b)", &Json::Null, &h).err().unwrap().message.contains("one relationship type"));
        // a cycle is refused before anything runs
        let e = translate("CREATE (n:Customer:X:_Entity)", &Json::Null, &h).err().unwrap();
        assert_eq!(e.code, "Neo.ClientError.Statement.SemanticError");
    }

    #[test]
    fn statements_are_classified_by_the_engines_parser() {
        assert!(!check("MATCH (n) RETURN n // CREATE (x)").unwrap());
        assert!(check("MATCH (n) SET n.x = 'CREATE'").unwrap());
        assert_eq!(check("MATCH (n RETURN n").err().unwrap().code, "Neo.ClientError.Statement.SyntaxError");
        let st = translate("CREATE CONSTRAINT c IF NOT EXISTS FOR (n:L) REQUIRE n.id IS UNIQUE", &Json::Null, &Hierarchy::default()).ok().unwrap();
        assert!(matches!(st.plan, Plan::Ddl(_)) && st.write);
        let st = translate("MATCH (n:L) RETURN n.id AS id, count(*)", &Json::Null, &Hierarchy::default()).ok().unwrap();
        assert_eq!(st.columns, vec!["id", "count(*)"]);
        let Plan::Cypher { ncols, .. } = st.plan else { panic!() };
        assert_eq!(ncols, 2);
    }
}
