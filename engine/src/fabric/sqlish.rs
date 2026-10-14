//! Fabric statements — a small SQL dialect for managing the fabric, and the way
//! a Cypher query enters from a PostgreSQL client.
//!
//! PostgreSQL offers an extension no hook to add top-level grammar (ADR-008), so
//! `CREATE DATASOURCE …` cannot be a statement the server parses. What it can be
//! is text that is *translated* to SQL the server does parse: this module is
//! that translation, and nothing else. `og_fabric_translate` returns the SQL;
//! the gateway in `sql-gateway/` asks for it and sends the result in the
//! client's place, and `og_fabric` runs it for callers that have no gateway.
//!
//! Keeping the grammar here rather than in the gateway means the regression
//! suite covers it, and the gateway stays a relay that knows only which
//! statements to ask about.
//!
//! ```text
//! CREATE DATASOURCE erp WITH ENGINE = 'postgres', PARAMETERS = {"host": …, "schema": "erp"}
//! DROP DATASOURCE erp            REFRESH DATASOURCE erp
//! SHOW DATASOURCES               SHOW TABLES FROM erp
//! CREATE GRAPH scm               SHOW GRAPHS              USE GRAPH scm
//! CREATE VIRTUAL NODE Item FROM erp.item KEY item_id [PROPERTIES (name, safety_stock AS safety)]
//! CREATE VIRTUAL RELATIONSHIP SHIPS FROM Shipment(ref_po_no) TO PurchaseOrder(po_id)
//! SHOW VIRTUAL NODES             SHOW VIRTUAL RELATIONSHIPS
//! DROP VIRTUAL NODE Item         DROP VIRTUAL RELATIONSHIP SHIPS
//! MATERIALIZE VIRTUAL NODE Customer
//! CREATE VIRTUAL NODE Score KEY supplier_id AS SELECT … FROM erp.a JOIN wms.b …
//! EXPLAIN FABRIC MATCH …
//! MATCH (s:Supplier) RETURN s.name          -- any Cypher query
//! ```
//!
//! `IN <graph>` may follow the name in any node or relationship statement;
//! without it the graph is the one `USE GRAPH` chose, or the only one there is.

use crate::catalog::types;
use crate::spiu::{lit, one};
use pgrx::prelude::*;
use pgrx::JsonB;
use serde_json::{Map, Value};

type R<T> = Result<T, String>;

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    /// A bare or double-quoted identifier, as written.
    Word(String),
    Str(String),
    Sym(char),
    /// A `{ … }` block, kept verbatim: datasource parameters are JSON.
    Json(String),
}

fn tokenize(src: &str) -> R<Vec<Tok>> {
    let cs: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '-' && cs.get(i + 1) == Some(&'-') {
            while i < cs.len() && cs[i] != '\n' {
                i += 1;
            }
        } else if c == '\'' {
            let mut s = String::new();
            i += 1;
            loop {
                match cs.get(i) {
                    None => return Err("unterminated string".into()),
                    Some('\'') if cs.get(i + 1) == Some(&'\'') => {
                        s.push('\'');
                        i += 2;
                    }
                    Some('\'') => {
                        i += 1;
                        break;
                    }
                    Some(ch) => {
                        s.push(*ch);
                        i += 1;
                    }
                }
            }
            out.push(Tok::Str(s));
        } else if c == '"' {
            let end = cs[i + 1..].iter().position(|&x| x == '"').ok_or("unterminated quoted identifier")?;
            out.push(Tok::Word(cs[i + 1..i + 1 + end].iter().collect()));
            i += end + 2;
        } else if c == '{' {
            // Balanced braces, minding strings inside the block.
            let (start, mut depth, mut in_str, mut quote) = (i, 0i32, false, '"');
            loop {
                match cs.get(i) {
                    None => return Err("unterminated { … } block".into()),
                    Some('\\') if in_str => i += 1,
                    Some(q @ ('"' | '\'')) if !in_str => {
                        in_str = true;
                        quote = *q;
                    }
                    Some(q) if in_str && *q == quote => in_str = false,
                    Some('{') if !in_str => depth += 1,
                    Some('}') if !in_str => depth -= 1,
                    _ => {}
                }
                i += 1;
                if depth == 0 {
                    break;
                }
            }
            out.push(Tok::Json(cs[start..i].iter().collect()));
        } else if c.is_alphanumeric() || c == '_' {
            let start = i;
            while i < cs.len() && (cs[i].is_alphanumeric() || cs[i] == '_') {
                i += 1;
            }
            out.push(Tok::Word(cs[start..i].iter().collect()));
        } else {
            out.push(Tok::Sym(c));
            i += 1;
        }
    }
    Ok(out)
}

struct P<'a> {
    t: Vec<Tok>,
    i: usize,
    /// The graph the caller is in (`og_cypher`'s argument), when there is one.
    graph: Option<&'a str>,
    params: &'a Map<String, Value>,
}

impl P<'_> {
    fn peek_kw(&self, kw: &str) -> bool {
        matches!(self.t.get(self.i), Some(Tok::Word(w)) if w.eq_ignore_ascii_case(kw))
    }
    fn kw(&mut self, kw: &str) -> bool {
        let hit = self.peek_kw(kw);
        if hit {
            self.i += 1;
        }
        hit
    }
    fn expect_kw(&mut self, kw: &str) -> R<()> {
        if self.kw(kw) {
            Ok(())
        } else {
            Err(format!("expected {kw}{}", self.near()))
        }
    }
    fn sym(&mut self, c: char) -> bool {
        let hit = self.t.get(self.i) == Some(&Tok::Sym(c));
        if hit {
            self.i += 1;
        }
        hit
    }
    fn expect_sym(&mut self, c: char) -> R<()> {
        if self.sym(c) {
            Ok(())
        } else {
            Err(format!("expected '{c}'{}", self.near()))
        }
    }
    fn word(&mut self, what: &str) -> R<String> {
        match self.t.get(self.i) {
            Some(Tok::Word(w)) => {
                self.i += 1;
                Ok(w.clone())
            }
            _ => Err(format!("expected {what}{}", self.near())),
        }
    }
    /// `name` or `schema.name`.
    fn qualified(&mut self, what: &str) -> R<String> {
        let mut s = self.word(what)?;
        while self.sym('.') {
            s.push('.');
            s.push_str(&self.word(what)?);
        }
        Ok(s)
    }
    fn near(&self) -> String {
        match self.t.get(self.i) {
            None => " at the end of the statement".into(),
            Some(Tok::Word(w)) => format!(", found '{w}'"),
            Some(Tok::Str(s)) => format!(", found '{s}'"),
            Some(Tok::Sym(c)) => format!(", found '{c}'"),
            Some(Tok::Json(_)) => ", found a { … } block".into(),
        }
    }
    fn end(&self) -> R<()> {
        if self.i == self.t.len() {
            Ok(())
        } else {
            Err(format!("unexpected text after the statement{}", self.near()))
        }
    }
    /// An optional `IN <graph>`, else the session's graph.
    fn graph(&mut self) -> R<String> {
        if self.kw("IN") {
            self.kw("GRAPH");
            let g = self.word("a graph name")?;
            return check_graph(&g);
        }
        default_graph(self.graph)
    }
}

fn check_graph(name: &str) -> R<String> {
    let known = one::<bool>("SELECT true FROM og_catalog.graph WHERE name = $1", &[name.into()])
        .ok()
        .flatten()
        .unwrap_or(false);
    if known {
        Ok(name.to_string())
    } else {
        Err(format!("graph '{name}' does not exist"))
    }
}

/// The graph a statement is about when it does not say: the caller's own, when
/// it arrived through `og_cypher`; otherwise the session's.
fn default_graph(given: Option<&str>) -> R<String> {
    match given {
        Some(g) => check_graph(g),
        None => current_graph(),
    }
}

/// The graph `USE GRAPH` set for this session, or the only graph in the database.
fn current_graph() -> R<String> {
    let set = one::<String>("SELECT nullif(current_setting('og.graph', true), '')", &[]).ok().flatten();
    if let Some(g) = set {
        return check_graph(&g);
    }
    let graphs: Vec<String> = Spi::connect(|c| {
        c.select("SELECT name::text FROM og_catalog.graph ORDER BY name", None, &[])
            .map(|rows| rows.filter_map(|r| r.get::<String>(1).ok().flatten()).collect())
            .unwrap_or_default()
    });
    match graphs.len() {
        1 => Ok(graphs[0].clone()),
        0 => Err("there is no graph yet: SELECT og_create_graph('name')".into()),
        _ => Err(format!("no graph selected: USE GRAPH <name> ({})", graphs.join(", "))),
    }
}

/// Dollar-quote a text that may itself contain dollar quotes.
fn dq(s: &str) -> String {
    let mut n = 0;
    loop {
        let tag = if n == 0 { "$og$".to_string() } else { format!("$og{n}$") };
        if !s.contains(&tag) {
            return format!("{tag}{s}{tag}");
        }
        n += 1;
    }
}

/// A `PARAMETERS` block as strict JSON. It may be written as JSON or as a
/// Cypher map — bare keys, single-quoted strings — and may name query
/// parameters (`password: $pw`), which is how a secret stays out of the
/// statement text and so out of logs and query history.
fn resolve_block(raw: &str, params: &Map<String, Value>) -> R<String> {
    let cs: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c == '"' {
            let start = i;
            i += 1;
            while i < cs.len() && cs[i] != '"' {
                if cs[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            out.extend(&cs[start..i.min(cs.len())]);
        } else if c == '\'' {
            let mut v = String::new();
            i += 1;
            loop {
                match cs.get(i) {
                    None => return Err("unterminated string in the parameter block".into()),
                    Some('\'') if cs.get(i + 1) == Some(&'\'') => {
                        v.push('\'');
                        i += 2;
                    }
                    Some('\'') => {
                        i += 1;
                        break;
                    }
                    Some(ch) => {
                        v.push(*ch);
                        i += 1;
                    }
                }
            }
            out.push_str(&Value::String(v).to_string());
        } else if c == '$' {
            let start = i + 1;
            i += 1;
            while i < cs.len() && (cs[i].is_alphanumeric() || cs[i] == '_') {
                i += 1;
            }
            let name: String = cs[start..i].iter().collect();
            let v = params.get(&name).ok_or(format!("parameter ${name} was not supplied"))?;
            out.push_str(&v.to_string());
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < cs.len() && (cs[i].is_alphanumeric() || cs[i] == '_') {
                i += 1;
            }
            let word: String = cs[start..i].iter().collect();
            let is_key = cs[i..].iter().find(|x| !x.is_whitespace()) == Some(&':');
            if is_key {
                out.push_str(&Value::String(word).to_string());
            } else {
                out.push_str(&word); // true, false, null
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    serde_json::from_str::<Map<String, Value>>(&out)
        .map_err(|e| format!("the parameter block is not a map of settings: {e}"))?;
    Ok(out)
}

/// The statement with its parameter block blanked — what is safe to keep in an
/// audit trail or to show back.
pub fn redacted(stmt: &str) -> String {
    match (stmt.find('{'), stmt.rfind('}')) {
        (Some(a), Some(b)) if a < b && first_words(stmt, 2).get(1).map(String::as_str) != Some("VIRTUAL") => {
            format!("{}{{…}}{}", &stmt[..a], &stmt[b + 1..])
        }
        _ => stmt.to_string(),
    }
}

/// True for a fabric statement that is not itself Cypher: what `og_cypher`
/// hands to this module instead of to its own parser. `USE` is left out — in
/// Cypher it belongs to the query that follows it.
pub fn is_statement(stmt: &str) -> bool {
    let s = trim_statement(stmt);
    let w = first_words(s, 2);
    let (w0, w1) = (w.first().map(String::as_str).unwrap_or(""), w.get(1).map(String::as_str).unwrap_or(""));
    match w0 {
        "CREATE" => {
            matches!(w1, "DATASOURCE" | "VIRTUAL" | "GRAPH")
                || (w1 == "DATABASE" && s.to_ascii_uppercase().split_whitespace().any(|x| x.starts_with("ENGINE")))
        }
        "DROP" => matches!(w1, "DATASOURCE" | "VIRTUAL"),
        "SHOW" => matches!(w1, "DATASOURCES" | "TABLES" | "VIRTUAL" | "GRAPHS"),
        "REFRESH" => w1 == "DATASOURCE",
        "MATERIALIZE" => true,
        "EXPLAIN" => w1 == "FABRIC",
        _ => false,
    }
}

/// Whether a fabric statement changes anything.
pub fn is_write(stmt: &str) -> bool {
    !matches!(first_words(trim_statement(stmt), 1).first().map(String::as_str), Some("SHOW" | "EXPLAIN"))
}

/// The result columns of a fabric statement, in order. Known from its opening
/// words alone, which is what lets `og_cypher_columns` stay a pure function.
pub fn columns(stmt: &str) -> Vec<String> {
    let w = first_words(trim_statement(stmt), 3);
    let w: Vec<&str> = w.iter().map(String::as_str).collect();
    let cols: &[&str] = match w.as_slice() {
        ["SHOW", "GRAPHS", ..] => &["graph", "types"],
        ["SHOW", "DATASOURCES", ..] => &["name", "engine", "host", "tables"],
        ["SHOW", "TABLES", ..] => &["table", "columns", "virtual_node"],
        ["SHOW", "VIRTUAL", "NODES"] => &["node", "source", "key", "properties", "storage"],
        ["SHOW", "VIRTUAL", ..] => &["relationship", "from", "to"],
        ["EXPLAIN", "FABRIC", ..] => &["step", "place", "operation", "rows", "detail"],
        _ => &["result"],
    };
    cols.iter().map(|c| c.to_string()).collect()
}

/// Leading whitespace and comments off, trailing semicolons off.
fn trim_statement(s: &str) -> &str {
    let mut s = s.trim();
    loop {
        if let Some(rest) = s.strip_prefix("--").or_else(|| s.strip_prefix("//")) {
            s = rest.split_once('\n').map_or("", |(_, r)| r).trim_start();
        } else if let Some(rest) = s.strip_prefix("/*") {
            s = rest.split_once("*/").map_or("", |(_, r)| r).trim_start();
        } else {
            break;
        }
    }
    s.trim_end_matches(|c: char| c == ';' || c.is_whitespace())
}

fn first_words(s: &str, n: usize) -> Vec<String> {
    s.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .take(n)
        .map(|w| w.to_ascii_uppercase())
        .collect()
}

/// Cypher's own clause openers. `CREATE (` is Cypher; `CREATE DATASOURCE` is not.
fn is_cypher(s: &str) -> bool {
    let w = first_words(s, 1);
    match w.first().map(String::as_str) {
        Some("MATCH" | "OPTIONAL" | "MERGE" | "UNWIND" | "RETURN" | "DETACH") => true,
        Some("CREATE") => s[6..].trim_start().starts_with('('),
        _ => false,
    }
}

/// A Cypher query as SQL with one column per `RETURN` item.
fn cypher_sql(graph: &str, q: &str) -> R<String> {
    let cols = one::<Vec<String>>("SELECT og_cypher_columns($1)", &[q.into()])
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let call = format!("og_cypher({}, {})", lit(graph), dq(q));
    if cols.is_empty() {
        return Ok(format!("SELECT r AS result FROM {call} r"));
    }
    // `->>` gives text, which is what a terminal wants; a nested value (a node,
    // a list) arrives as its JSON text.
    let list: Vec<String> = cols.iter().map(|c| format!("r ->> {} AS {}", lit(c), ident_always(c))).collect();
    Ok(format!("SELECT {} FROM {call} r", list.join(", ")))
}

/// Quote unconditionally: a column called `s.name` or `count(po)` is not an identifier.
fn ident_always(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// A `CREATE VIRTUAL NODE … AS <query>` split at its `AS`, if it is one.
fn split_as_query(s: &str) -> Option<(&str, &str)> {
    let w = first_words(s, 3);
    if w != ["CREATE", "VIRTUAL", "NODE"] {
        return None;
    }
    let upper = s.to_ascii_uppercase();
    let mut from = 0;
    while let Some(at) = upper[from..].find("AS") {
        let a = from + at;
        let before = upper[..a].chars().last().map_or(true, |c| c.is_whitespace());
        let rest = upper[a + 2..].trim_start();
        let after = upper[a + 2..].starts_with(|c: char| c.is_whitespace());
        if before && after && (rest.starts_with("SELECT") || rest.starts_with("WITH") || rest.starts_with('(')) {
            return Some((s[..a].trim_end(), s[a + 2..].trim()));
        }
        from = a + 2;
    }
    None
}

fn translate(stmt: &str, graph: Option<&str>, params: &Map<String, Value>) -> R<String> {
    let s = trim_statement(stmt);
    if s.is_empty() {
        return Err("empty statement".into());
    }
    if is_cypher(s) {
        return cypher_sql(&default_graph(graph)?, s);
    }
    if let Some((head, query)) = split_as_query(s) {
        // CREATE VIRTUAL NODE <Type> [IN g] KEY <col> AS <query>
        let mut p = P { t: tokenize(head)?, i: 3, graph, params };
        let ty = p.word("a node type name")?;
        let g = p.graph()?;
        p.expect_kw("KEY")?;
        let key = p.word("the key column")?;
        p.end()?;
        return Ok(format!(
            "SELECT og_create_view_node({}, {}, {}, {}) AS result",
            lit(&g), lit(&ty), lit(&key), dq(query)
        ));
    }
    let head = first_words(s, 2);
    if head == ["EXPLAIN", "FABRIC"] {
        let at = s.to_ascii_uppercase().find("FABRIC").unwrap() + 6;
        let q = s[at..].trim();
        return Ok(format!(
            "SELECT step, place, operation, rows, detail FROM og_fabric_explain({}, {}, true, {}::jsonb)",
            lit(&default_graph(graph)?),
            dq(q),
            lit(&Value::Object(params.clone()).to_string())
        ));
    }

    let mut p = P { t: tokenize(s)?, i: 0, graph, params };
    let sql = if p.kw("CREATE") {
        if p.kw("GRAPH") {
            let name = p.word("a graph name")?;
            // Created, and chosen for the rest of a SQL session.
            format!(
                "SELECT format('graph %s created', {n}) AS result \
                 FROM og_create_graph({n}), set_config('og.graph', {n}, false)",
                n = lit(&name)
            )
        } else if p.kw("DATASOURCE") || p.kw("DATABASE") {
            let name = p.word("a datasource name")?;
            p.kw("WITH");
            p.expect_kw("ENGINE")?;
            p.sym('=');
            let engine = match p.t.get(p.i) {
                Some(Tok::Str(e)) | Some(Tok::Word(e)) => e.clone(),
                _ => return Err(format!("expected an engine name{}", p.near())),
            };
            p.i += 1;
            p.sym(',');
            if !(p.kw("PARAMETERS") || p.kw("OPTIONS")) {
                return Err(format!("expected PARAMETERS = {{ … }}{}", p.near()));
            }
            p.sym('=');
            let json = match p.t.get(p.i) {
                Some(Tok::Json(j)) => resolve_block(j, p.params)?,
                _ => return Err(format!("expected PARAMETERS = {{ … }}{}", p.near())),
            };
            p.i += 1;
            format!(
                "SELECT og_create_datasource({}, {}, {}::jsonb) AS result",
                lit(&name), lit(&engine), lit(&json)
            )
        } else if p.kw("VIRTUAL") {
            if p.kw("NODE") {
                let ty = p.word("a node type name")?;
                let graph = p.graph()?;
                p.expect_kw("FROM")?;
                let rel = p.qualified("schema.table")?;
                if !rel.contains('.') {
                    return Err(format!("FROM needs schema.table (a datasource's table, or a view), got '{rel}'"));
                }
                p.expect_kw("KEY")?;
                let key = p.word("the key column")?;
                let mut pmap = "NULL".to_string();
                if p.kw("PROPERTIES") {
                    p.expect_sym('(')?;
                    let mut m = Map::new();
                    loop {
                        let col = p.word("a column name")?;
                        let prop = if p.kw("AS") { p.word("a property name")? } else { col.clone() };
                        m.insert(prop, Value::String(col));
                        if !p.sym(',') {
                            break;
                        }
                    }
                    p.expect_sym(')')?;
                    pmap = format!("{}::jsonb", lit(&Value::Object(m).to_string()));
                }
                format!(
                    "SELECT format('virtual node %s ← %s (%s properties)', j->>'type', {src}, \
                     (SELECT count(*) FROM jsonb_object_keys(j->'properties'))) AS result \
                     FROM og_map_relation({g}, {t}, {src}, {k}, {pmap}) j",
                    g = lit(&graph), t = lit(&ty), src = lit(&rel), k = lit(&key)
                )
            } else if p.kw("RELATIONSHIP") || p.kw("REL") {
                let name = p.word("a relationship name")?;
                let graph = p.graph()?;
                p.expect_kw("FROM")?;
                let (a, ap) = endpoint(&mut p)?;
                p.expect_kw("TO")?;
                let (b, bp) = endpoint(&mut p)?;
                format!(
                    "SELECT format('virtual relationship (:%s)-[:%s]->(:%s) on %s.%s = %s.%s', \
                     {a}, {n}, {b}, {a}, {ap}, {b}, {bp}) AS result \
                     FROM og_create_virtual_rel({g}, {n}, {a}, {ap}, {b}, {bp})",
                    g = lit(&graph), n = lit(&name), a = lit(&a), ap = lit(&ap), b = lit(&b), bp = lit(&bp)
                )
            } else {
                return Err(format!("expected NODE or RELATIONSHIP after CREATE VIRTUAL{}", p.near()));
            }
        } else {
            return Err("not a fabric statement".into());
        }
    } else if p.kw("DROP") {
        if p.kw("DATASOURCE") {
            let name = p.word("a datasource name")?;
            format!("SELECT og_drop_datasource({}) AS result", lit(&name))
        } else if p.kw("VIRTUAL") {
            if p.kw("NODE") {
                let ty = p.word("a node type name")?;
                let graph = p.graph()?;
                format!("SELECT og_drop_virtual({}, {}, false) AS result", lit(&graph), lit(&ty))
            } else if p.kw("RELATIONSHIP") || p.kw("REL") {
                let name = p.word("a relationship name")?;
                let graph = p.graph()?;
                format!("SELECT og_drop_virtual({}, {}, true) AS result", lit(&graph), lit(&name))
            } else {
                return Err(format!("expected NODE or RELATIONSHIP after DROP VIRTUAL{}", p.near()));
            }
        } else {
            return Err("not a fabric statement".into());
        }
    } else if p.kw("REFRESH") {
        p.expect_kw("DATASOURCE")?;
        let name = p.word("a datasource name")?;
        format!("SELECT og_refresh_datasource({}) AS result", lit(&name))
    } else if p.kw("MATERIALIZE") {
        p.kw("VIRTUAL");
        p.expect_kw("NODE")?;
        let ty = p.word("a node type name")?;
        let graph = p.graph()?;
        format!(
            "SELECT format('node %s materialized: %s rows copied, now stored and writable', {t}, n) AS result \
             FROM og_materialize_mapping({g}, {t}) n",
            g = lit(&graph), t = lit(&ty)
        )
    } else if p.kw("USE") {
        p.kw("GRAPH");
        let g = check_graph(&p.word("a graph name")?)?;
        format!("SELECT set_config('og.graph', {}, false) AS graph", lit(&g))
    } else if p.kw("SHOW") {
        if p.kw("GRAPHS") {
            "SELECT g.name AS graph, (SELECT count(*) FROM og_catalog.type t WHERE t.graph_id = g.graph_id) AS types \
               FROM og_catalog.graph g ORDER BY 1"
                .to_string()
        } else if p.kw("DATASOURCES") {
            "SELECT d.name, d.engine, d.options->>'host' AS host, \
                    (SELECT count(*) FROM pg_foreign_table ft JOIN pg_foreign_server s ON s.oid = ft.ftserver \
                      WHERE s.srvname = d.server_name) AS tables \
               FROM og_catalog.datasource d ORDER BY d.name"
                .to_string()
        } else if p.kw("TABLES") {
            if !(p.kw("FROM") || p.kw("IN")) {
                return Err(format!("expected FROM <datasource>{}", p.near()));
            }
            let ds = p.word("a datasource name")?;
            let known = one::<bool>("SELECT true FROM og_catalog.datasource WHERE name = $1", &[ds.as_str().into()])
                .ok()
                .flatten()
                .unwrap_or(false);
            if !known {
                return Err(format!("datasource '{ds}' is not registered"));
            }
            // Which node type, if any, reads each table directly.
            format!(
                "SELECT n.nspname || '.' || c.relname AS \"table\", \
                        (SELECT count(*) FROM pg_attribute a WHERE a.attrelid = c.oid AND a.attnum > 0 \
                            AND NOT a.attisdropped) AS columns, \
                        (SELECT string_agg(t.name, ', ' ORDER BY t.name) FROM og_catalog.mapping m \
                           JOIN og_catalog.type t USING (type_id) \
                          WHERE to_regclass(m.source_table) = c.oid) AS virtual_node \
                   FROM pg_foreign_table ft JOIN pg_foreign_server s ON s.oid = ft.ftserver \
                   JOIN og_catalog.datasource d ON d.server_name = s.srvname \
                   JOIN pg_class c ON c.oid = ft.ftrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
                  WHERE d.name = {} ORDER BY 1",
                lit(&ds)
            )
        } else if p.kw("VIRTUAL") {
            if p.kw("NODES") {
                let graph = p.graph()?;
                format!(
                    "SELECT t.name AS node, replace(m.source_table, chr(34), '') AS source, m.id_column AS key, \
                            (SELECT count(*) FROM og_catalog.property p WHERE p.type_id = t.type_id) AS properties, \
                            CASE WHEN c.relkind = 'v' THEN 'virtual' ELSE 'materialized' END AS storage \
                       FROM og_catalog.mapping m JOIN og_catalog.type t USING (type_id) \
                       JOIN og_catalog.graph g USING (graph_id) \
                       JOIN pg_class c ON c.oid = to_regclass(t.storage_table) \
                      WHERE g.name = {} ORDER BY 1",
                    lit(&graph)
                )
            } else if p.kw("RELATIONSHIPS") || p.kw("RELS") {
                let graph = p.graph()?;
                format!(
                    "SELECT r.name AS relationship, s.name || '.' || v.src_prop AS \"from\", \
                            d.name || '.' || v.dst_prop AS \"to\" \
                       FROM og_catalog.virtual_rel v JOIN og_catalog.type r ON r.type_id = v.type_id \
                       JOIN og_catalog.type s ON s.type_id = v.src_type \
                       JOIN og_catalog.type d ON d.type_id = v.dst_type \
                       JOIN og_catalog.graph g ON g.graph_id = r.graph_id \
                      WHERE g.name = {} ORDER BY 1",
                    lit(&graph)
                )
            } else {
                return Err(format!("expected NODES or RELATIONSHIPS after SHOW VIRTUAL{}", p.near()));
            }
        } else {
            return Err("not a fabric statement".into());
        }
    } else {
        return Err("not a fabric statement".into());
    };
    p.end()?;
    Ok(sql)
}

/// `Type(property)`.
fn endpoint(p: &mut P) -> R<(String, String)> {
    let ty = p.word("a node type name")?;
    p.expect_sym('(')?;
    let prop = p.word("a property name")?;
    p.expect_sym(')')?;
    Ok((ty, prop))
}

/// The SQL a fabric statement or a Cypher query stands for. Translating has no
/// effect of its own; running the result does.
#[pg_extern]
fn og_fabric_translate(statement: &str, params: default!(JsonB, "'{}'")) -> String {
    let p = match params.0 {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    translate(statement, None, &p).unwrap_or_else(|e| error!("fabric statement: {e}"))
}

/// Run a fabric statement or a Cypher query, one jsonb object per result row —
/// for callers with no gateway in front of them.
#[pg_extern]
fn og_fabric(statement: &str, params: default!(JsonB, "'{}'")) -> SetOfIterator<'static, JsonB> {
    let p = match params.0 {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    SetOfIterator::new(run(statement, None, &p).into_iter().map(JsonB))
}

/// Translate and run, on behalf of `og_cypher` (which passes its graph) or of
/// `og_fabric`. The statement is recorded with its parameter block blanked.
pub fn run(statement: &str, graph: Option<&str>, params: &Map<String, Value>) -> Vec<Value> {
    let started = std::time::Instant::now();
    let shown = format!("[{}] {}", graph.unwrap_or("-"), redacted(statement));
    let audit = |rows: i64, err: Option<&str>| {
        Spi::run_with_args(
            "INSERT INTO og_data.og_audit (query, lang, rows_out, duration_ms, error_code)
             VALUES ($1, 'fabric', $2, $3, $4)",
            &[
                shown.as_str().into(),
                rows.into(),
                (started.elapsed().as_secs_f64() * 1000.0).into(),
                err.map(|e| e.chars().take(200).collect::<String>()).into(),
            ],
        )
        .ok();
    };
    let sql = match translate(statement, graph, params) {
        Ok(sql) => sql,
        Err(e) => {
            audit(0, Some(&e));
            error!("fabric statement: {e}")
        }
    };
    let rows: Vec<Value> = Spi::connect_mut(|client| {
        client
            .update(&format!("SELECT to_jsonb(t) FROM ({sql}) t"), None, &[])
            .unwrap_or_else(|e| error!("{e}"))
            .filter_map(|r| r.get::<JsonB>(1).ok().flatten().map(|j| j.0))
            .collect()
    });
    audit(rows.len() as i64, None);
    rows
}

/// The SQL a fabric statement stands for, safe to show: a datasource's
/// parameters are left out of it.
pub fn shown_sql(statement: &str, graph: &str) -> R<String> {
    if first_words(trim_statement(statement), 2).get(1).map(String::as_str) == Some("DATASOURCE")
        && first_words(statement, 1).first().map(String::as_str) == Some("CREATE")
    {
        return Ok("-- CREATE DATASOURCE runs og_create_datasource(name, engine, parameters); \
                   the parameters are not shown"
            .to_string());
    }
    translate(statement, Some(graph), &Map::new())
}

/// A node type defined by a query: the query becomes a view, the view the type.
/// What `CREATE VIRTUAL NODE … KEY … AS SELECT …` runs.
#[pg_extern]
fn og_create_view_node(graph: &str, type_name: &str, key_column: &str, query: &str) -> String {
    // One statement, and a query: SPI would run whatever followed a `;`.
    let mut quote: Option<char> = None;
    for c in query.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '\'' || c == '"' => quote = Some(c),
            None if c == ';' => error!("the query of a virtual node is a single SELECT; found ';'"),
            None => {}
        }
    }
    let view = format!("og_views.{}", crate::spiu::ident(&format!("{}__{}", graph, type_name.to_lowercase())));
    Spi::run("CREATE SCHEMA IF NOT EXISTS og_views").expect("schema");
    Spi::run(&format!("CREATE OR REPLACE VIEW {view} AS {query}"))
        .unwrap_or_else(|e| error!("the query of virtual node '{type_name}' is not valid: {e}"));
    let j = super::map_relation(
        graph,
        type_name,
        "og_views",
        &format!("{}__{}", graph, type_name.to_lowercase()),
        key_column,
        None,
    );
    let n = j.0.get("properties").and_then(Value::as_object).map_or(0, |m| m.len());
    format!("virtual node {type_name} ← query ({n} properties)")
}

/// Remove a virtual node type or a virtual relationship. A node type that
/// relationships still join on is kept.
#[pg_extern]
fn og_drop_virtual(graph: &str, name: &str, relationship: bool) -> String {
    let gid = types::graph_id(graph);
    let tid = types::try_type_id(gid, name).unwrap_or_else(|| error!("'{name}' does not exist in graph '{graph}'"));
    if relationship {
        let n = one::<i64>(
            "WITH d AS (DELETE FROM og_catalog.virtual_rel WHERE type_id = $1 RETURNING 1) SELECT count(*) FROM d",
            &[tid.into()],
        )
        .ok()
        .flatten()
        .unwrap_or(0);
        if n == 0 {
            error!("'{name}' is not a virtual relationship");
        }
        Spi::run_with_args("SELECT og_drop_type($1, $2, false)", &[graph.into(), name.into()])
            .unwrap_or_else(|e| error!("{e}"));
        return format!("virtual relationship {name} dropped");
    }
    if super::virtual_source(tid).is_none() {
        error!("'{name}' is not a virtual node type");
    }
    let used = one::<String>(
        "SELECT string_agg(r.name, ', ' ORDER BY r.name) FROM og_catalog.virtual_rel v
           JOIN og_catalog.type r ON r.type_id = v.type_id WHERE v.src_type = $1 OR v.dst_type = $1",
        &[tid.into()],
    )
    .ok()
    .flatten();
    if let Some(rels) = used {
        error!("virtual relationships still join on '{name}' ({rels}); drop them first");
    }
    let table = types::storage_table(tid).unwrap_or_default();
    Spi::run(&format!("DROP VIEW IF EXISTS {table} CASCADE")).unwrap_or_else(|e| error!("{e}"));
    Spi::run_with_args("SELECT og_drop_type($1, $2, false)", &[graph.into(), name.into()])
        .unwrap_or_else(|e| error!("{e}"));
    // A node defined by a query owned its view; nothing else should read it.
    Spi::run(&format!(
        "DROP VIEW IF EXISTS og_views.{}",
        crate::spiu::ident(&format!("{}__{}", graph, name.to_lowercase()))
    ))
    .ok();
    format!("virtual node {name} dropped")
}
