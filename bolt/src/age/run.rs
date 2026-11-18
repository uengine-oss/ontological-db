//! A session's statements on the AGE backend (`OG_BOLT_BACKEND=age`).
//!
//! A child of `session.rs`: the Bolt side — messages, transactions, the
//! cancel watch, failures — is the session's own and unchanged; this is what
//! takes the place of `og_cypher_check()`, `og_cypher_columns()`,
//! `og_cypher()` and `og_cypher_stats()`.

use super::{map, string, tx_settings, tx_timeout, Failure, Session, State, Value, Watch};
use crate::age::{self, agtype, Counts, Plan, Stats};
use crate::Config;
use postgres::error::SqlState;
use postgres::Client;
use std::net::TcpStream;

/// After the PostgreSQL login: AGE's library, its schema on the path, the
/// deployment's time limit. Returns the session's limit in milliseconds.
pub(super) fn hello(client: &mut Client, config: &Config) -> Result<i64, Failure> {
    // Preloaded (`shared_preload_libraries`, as the Supabase image does) there
    // is nothing to do; elsewhere AGE's parser hook needs the library loaded,
    // which only a superuser may do.
    if config.age.as_ref().is_some_and(|a| a.load) {
        client.batch_execute("LOAD 'age'").map_err(|e| Failure::from_pg(&e))?;
    }
    client
        .batch_execute("SET search_path = ag_catalog, \"$user\", public")
        .map_err(|e| Failure::from_pg(&e))?;
    if let Some(limit) = &config.statement_timeout {
        client
            .execute("SELECT set_config('statement_timeout', $1, false)", &[limit])
            .map_err(|e| Failure::from_pg(&e))?;
    }
    Ok(client
        .query_one("SELECT setting::int8 FROM pg_settings WHERE name = 'statement_timeout'", &[])
        .map(|r| r.get::<_, i64>(0))
        .unwrap_or(0))
}

/// A PostgreSQL error from AGE, under the code a Neo4j driver expects.
fn failure(e: &postgres::Error, interrupt: Option<super::Interrupt>, limit_ms: i64) -> Failure {
    if e.code() == Some(&SqlState::QUERY_CANCELED) {
        return Failure::from_query(e, interrupt, limit_ms);
    }
    let message = super::pg_message(e);
    match age::error_code(e.code().map(|c| c.code()), &message) {
        Some(code) => Failure::client(code, message),
        None => Failure::from_pg(e),
    }
}

/// What one statement produced.
pub(crate) struct Outcome {
    pub records: Vec<Value>,
    pub stats: Option<Stats>,
}

impl Session<'_> {
    pub(super) fn age_run(
        &mut self,
        query: &str,
        plan_only: bool,
        params: &Value,
        extra: &Value,
        stream: &mut TcpStream,
    ) -> Result<Value, Failure> {
        let mut pjson = super::to_json(params).map_err(|e| Failure::client("Neo.ClientError.Statement.TypeError", e))?;
        if pjson.is_null() {
            pjson = serde_json::Value::Object(serde_json::Map::new());
        }
        if plan_only {
            // EXPLAIN / PROFILE: classified, not run — as on the default backend.
            let write = age::check(query).map_err(|r| Failure::client(&r.code, r.message))?;
            self.qtype = if write { "w" } else { "r" };
            self.fields = Vec::new();
            self.pending = Vec::new();
            self.cursor = 0;
            self.state = State::Streaming;
            return Ok(map(vec![("fields", Value::List(Vec::new())), ("t_first", Value::Int(0)), ("qid", Value::Int(-1))]));
        }
        let st = age::translate(query, &pjson).map_err(|r| Failure::client(&r.code, r.message))?;
        self.qtype = if st.write { "w" } else { "r" };
        if let serde_json::Value::Object(m) = &mut pjson {
            for (k, v) in &st.extra {
                m.insert(k.clone(), v.clone());
            }
        }

        // Settings and the limit, exactly as `run_query` takes them.
        let (own, limit_ms) = if self.in_tx {
            (Vec::new(), self.tx_timeout_ms.unwrap_or(self.timeout_ms))
        } else {
            let mut own = tx_settings(extra);
            let limit = tx_timeout(extra);
            if let Some(ms) = limit {
                own.push(("og.statement_timeout".into(), ms.to_string()));
            }
            (own, limit.unwrap_or(self.timeout_ms))
        };

        // SHOW GRAPHS / CREATE GRAPH are about the graphs, not in one.
        if !matches!(st.plan, Plan::Graphs(_)) {
            self.age_graph()?;
        }

        // A write runs in a transaction even when the client sent it on its
        // own: the counters are the transaction's (`pg_stat_xact_*`).
        let wrap = !self.in_tx && (!own.is_empty() || st.write || matches!(st.plan, Plan::Steps(_) | Plan::Reach(_)));
        if wrap {
            self.sql("BEGIN")?;
            if let Err(f) = self.apply_settings(&own) {
                let _ = self.sql("ROLLBACK");
                return Err(f);
            }
        }
        let done = self.age_execute(&st, &pjson, limit_ms, stream);
        let out = match (done, wrap) {
            (Ok(o), true) => {
                self.sql("COMMIT")?;
                o
            }
            (Ok(o), false) => o,
            (Err(f), true) => {
                let _ = self.sql("ROLLBACK");
                return Err(f);
            }
            (Err(f), false) => return Err(f),
        };
        self.age_stats = out.stats.map(|s| super::to_bolt(&s.to_json()));
        self.fields = st.columns.clone();
        self.pending = out.records;
        self.cursor = 0;
        self.state = State::Streaming;
        Ok(map(vec![
            ("fields", Value::List(st.columns.into_iter().map(Value::String).collect())),
            ("t_first", Value::Int(0)),
            ("qid", Value::Int(-1)),
        ]))
    }

    /// The session's graph exists — or is made, when the deployment says so.
    fn age_graph(&mut self) -> Result<(), Failure> {
        if self.age_graphs.contains(&self.graph) {
            return Ok(());
        }
        let graph = self.graph.clone();
        // AGE's graph names are identifiers; anything else would be a schema
        // name to quote, and AGE refuses most of them anyway.
        let ok = graph.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && graph.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !ok {
            return Err(Failure::client(
                "Neo.ClientError.Database.DatabaseNotFound",
                format!("'{graph}' is not a graph name AGE takes (letters, digits and _, not starting with a digit)"),
            ));
        }
        let create = self.config.age.as_ref().is_some_and(|a| a.create_graphs);
        let pg = self.client()?;
        let exists = pg.query_opt(age::GRAPH_EXISTS, &[&graph]).map_err(|e| Failure::from_pg(&e))?.is_some();
        if !exists {
            if !create {
                return Err(Failure::client(
                    "Neo.ClientError.Database.DatabaseNotFound",
                    format!("graph \"{graph}\" does not exist (OG_BOLT_AGE_CREATE_GRAPHS is off)"),
                ));
            }
            pg.execute(age::CREATE_GRAPH, &[&graph]).map_err(|e| failure(&e, None, 0))?;
        }
        self.age_graphs.insert(graph);
        Ok(())
    }

    fn age_counts(&mut self) -> Result<Counts, Failure> {
        let graph = self.graph.clone();
        let pg = self.client()?;
        let row = pg.query_one(age::COUNTS, &[&graph]).map_err(|e| Failure::from_pg(&e))?;
        Ok(Counts::from_row(&row))
    }

    fn age_execute(
        &mut self,
        st: &age::Statement,
        params: &serde_json::Value,
        limit_ms: i64,
        stream: &mut TcpStream,
    ) -> Result<Outcome, Failure> {
        let graph = self.graph.clone();
        match &st.plan {
            Plan::Cypher { text, ncols } => {
                let before = if st.write { Some(self.age_counts()?) } else { None };
                let sql = age::cypher_sql(&graph, text, *ncols);
                let rows = self.age_query(&sql, params, limit_ms, stream)?;
                let records = rows_to_records(&rows, *ncols)?;
                let stats = match before {
                    Some(b) => Some(Stats::from_counts(self.age_counts()?.minus(b))),
                    None => None,
                };
                Ok(Outcome { records, stats })
            }
            Plan::Ddl(d) => {
                let pg = self.client()?;
                let stats = age::ddl::run(pg, &graph, d).map_err(|f| Failure::client(f.code, f.message))?;
                Ok(Outcome { records: Vec::new(), stats: Some(stats) })
            }
            Plan::Procedure(p) => {
                let sql = p.sql(&graph);
                let width = p.columns().len();
                let pg = self.client()?;
                let rows = pg.query(sql.as_str(), &[]).map_err(|e| failure(&e, None, limit_ms))?;
                let records = rows
                    .iter()
                    .map(|r| {
                        let v: String = r.get(0);
                        Value::List(vec![string(v); width])
                    })
                    .collect();
                Ok(Outcome { records, stats: None })
            }
            Plan::Graphs(g) => {
                if let age::GraphStatement::Create(name) = g {
                    let ok = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                    if !ok {
                        return Err(Failure::client(
                            "Neo.ClientError.Statement.ArgumentError",
                            format!("'{name}' is not a graph name AGE takes (letters, digits and _)"),
                        ));
                    }
                }
                let pg = self.client()?;
                let rows = pg.simple_query(&g.sql()).map_err(|e| failure(&e, None, limit_ms))?;
                let mut records = Vec::new();
                for m in rows {
                    if let postgres::SimpleQueryMessage::Row(r) = m {
                        records.push(match g {
                            age::GraphStatement::Show => Value::List(vec![
                                string(r.get(0).unwrap_or("").to_string()),
                                Value::Int(r.get(1).and_then(|t| t.parse().ok()).unwrap_or(0)),
                            ]),
                            age::GraphStatement::Create(n) => Value::List(vec![string(format!("graph {n} created"))]),
                        });
                    }
                }
                Ok(Outcome { records, stats: None })
            }
            Plan::Reach(r) => {
                let before = if st.write { Some(self.age_counts()?) } else { None };
                let rows = {
                    let mut x = Steps { sess: &mut *self, stream: &mut *stream, limit_ms };
                    age::reach::run(&mut x, r, params).map_err(|r| Failure::client(&r.code, r.message))?
                };
                let stats = match before {
                    Some(b) => Some(Stats::from_counts(self.age_counts()?.minus(b))),
                    None => None,
                };
                let records = if r.ncols == 0 {
                    Vec::new()
                } else {
                    rows.iter().map(|row| Value::List(row.iter().map(agtype::Ag::to_bolt).collect())).collect()
                };
                Ok(Outcome { records, stats })
            }
            Plan::Steps(steps) => {
                let before = self.age_counts()?;
                let (rows, adjust) = {
                    let mut x = Steps { sess: &mut *self, stream: &mut *stream, limit_ms };
                    age::split::run(&mut x, steps, params).map_err(|r| Failure::client(&r.code, r.message))?
                };
                let mut stats = Stats::from_counts(self.age_counts()?.minus(before));
                stats.add(&adjust);
                let records = if steps.ncols == 0 {
                    Vec::new()
                } else {
                    rows.iter().map(|r| Value::List(r.iter().map(agtype::Ag::to_bolt).collect())).collect()
                };
                Ok(Outcome { records, stats: if st.write { Some(stats) } else { None } })
            }
        }
    }

    /// Run one `cypher()` call with the watch on, as `run_query` runs
    /// `og_cypher()`.
    pub(crate) fn age_query(
        &mut self,
        sql: &str,
        params: &serde_json::Value,
        limit_ms: i64,
        stream: &mut TcpStream,
    ) -> Result<Vec<postgres::Row>, Failure> {
        let p = agtype::AgParam(agtype::params_text(params));
        let trace = self.config.age.as_ref().is_some_and(|a| a.trace);
        let started = std::time::Instant::now();
        if trace {
            eprintln!("age-trace > {sql} -- params {}", p.0);
        }
        let pg = self.client()?;
        let watch = Watch::start(stream, pg.cancel_token(), limit_ms);
        let result = pg.query(sql, &[&p]);
        let seen = watch.finish(stream);
        if trace {
            eprintln!(
                "age-trace < {} ms {}",
                started.elapsed().as_millis(),
                match &result {
                    Ok(r) => format!("{} rows", r.len()),
                    Err(e) => format!("error {}", super::pg_message(e)),
                }
            );
        }
        self.early.extend(seen.bytes);
        result.map_err(|e| failure(&e, seen.interrupt, limit_ms))
    }

}

/// The session as the split steps see it.
struct Steps<'s, 'a> {
    sess: &'s mut Session<'a>,
    stream: &'s mut TcpStream,
    limit_ms: i64,
}

fn refusal(f: Failure) -> age::Refusal {
    age::Refusal { code: f.code, message: f.message }
}

impl age::split::Exec for Steps<'_, '_> {
    fn cypher(&mut self, text: &str, ncols: usize, params: &serde_json::Value) -> Result<Vec<Vec<agtype::Ag>>, age::Refusal> {
        let graph = self.sess.graph.clone();
        let sql = age::cypher_sql(&graph, text, ncols);
        let rows = self.sess.age_query(&sql, params, self.limit_ms, self.stream).map_err(refusal)?;
        if ncols == 0 {
            return Ok(Vec::new());
        }
        rows.iter().map(|r| row_values(r, ncols).map_err(refusal)).collect()
    }

    fn sql(&mut self, sql: &str) -> Result<Vec<Vec<Option<String>>>, age::Refusal> {
        let pg = self.sess.client().map_err(refusal)?;
        let rows = pg.simple_query(sql).map_err(|e| refusal(failure(&e, None, 0)))?;
        let mut out = Vec::new();
        for m in rows {
            if let postgres::SimpleQueryMessage::Row(r) = m {
                out.push((0..r.len()).map(|i| r.get(i).map(str::to_string)).collect());
            }
        }
        Ok(out)
    }

    fn graph(&self) -> String {
        self.sess.graph.clone()
    }
}

/// Rows of agtype columns as Bolt records.
pub(crate) fn rows_to_records(rows: &[postgres::Row], ncols: usize) -> Result<Vec<Value>, Failure> {
    if ncols == 0 {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(Value::List(row_values(row, ncols)?.iter().map(agtype::Ag::to_bolt).collect()));
    }
    Ok(out)
}

/// One row's agtype columns, read.
pub(crate) fn row_values(row: &postgres::Row, ncols: usize) -> Result<Vec<agtype::Ag>, Failure> {
    let mut vals = Vec::with_capacity(ncols);
    for i in 0..ncols {
        let raw: Option<agtype::AgText> =
            row.try_get(i).map_err(|e| Failure::client("Neo.DatabaseError.General.UnknownError", e.to_string()))?;
        vals.push(match raw {
            None => agtype::Ag::Null,
            Some(t) => agtype::parse(&t.0).map_err(|e| Failure::client("Neo.DatabaseError.General.UnknownError", e))?,
        });
    }
    Ok(vals)
}
