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
        if has_zone_id(params) {
            // A DateTime with a zone id becomes its instant (SUITES-ON-AGE B9).
            let mut seen: std::collections::HashMap<(String, String), String> = std::collections::HashMap::new();
            pjson = age_params(params, &mut |local: &str, zone: &str| {
                let key = (local.to_string(), zone.to_string());
                if let Some(t) = seen.get(&key) {
                    return Ok(t.clone());
                }
                let t = self.zone_instant(local, zone)?;
                seen.insert(key, t.clone());
                Ok(t)
            })?;
        }
        // The label hierarchy, for a statement that needs it (`hierarchy.rs`):
        // one catalog read, in the client's transaction if it has one.
        let parsed = age::parse(query);
        let hier = if parsed.needs_hierarchy() {
            self.age_graph()?;
            self.age_hierarchy(stream)?
        } else {
            age::hierarchy::Hierarchy::default()
        };
        let st = age::translate_parsed(parsed, &pjson, &hier).map_err(|r| Failure::client(&r.code, r.message))?;
        self.qtype = if st.write { "w" } else { "r" };
        if self.config.age.as_ref().is_some_and(|a| a.trace) {
            eprintln!("age-trace plan {}{}", st.plan.kind(), if st.write { " write" } else { "" });
        }
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
        // A MERGE key guarded against null that was null fails as a division
        // by zero; Neo4j's answer is its null-merge error (N7).
        let done = self.age_execute(&st, &pjson, limit_ms, stream).map_err(|f| {
            match age::null_merge_refusal(&f.code, &f.message, &st.merge_nulls) {
                Some(r) => Failure::client(&r.code, r.message),
                None => f,
            }
        });
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

    /// A local date-time in a zone (IANA name) as the instant, written as
    /// [`age::cypher::canonical_datetime`] writes one: PostgreSQL's zone
    /// database does the work (`timezone(zone, timestamp)` reads the wall
    /// time there; a time that falls in a gap or twice is resolved as
    /// PostgreSQL resolves it). A zone PostgreSQL does not know is a TypeError.
    fn zone_instant(&mut self, local: &str, zone: &str) -> Result<String, Failure> {
        let pg = self.client()?;
        let row = pg
            .query_one(ZONE_INSTANT, &[&local, &zone])
            .map_err(|e| Failure::client("Neo.ClientError.Statement.TypeError", super::pg_message(&e)))?;
        let utc: String = row.get(0);
        age::cypher::canonical_datetime(&format!("{utc}Z"))
            .ok_or_else(|| Failure::client("Neo.ClientError.Statement.TypeError", format!("{local}[{zone}] is not a date-time")))
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

    /// The graph's label hierarchy, read from the catalog now.
    fn age_hierarchy(&mut self, stream: &mut TcpStream) -> Result<age::hierarchy::Hierarchy, Failure> {
        let trace = self.config.age.as_ref().is_some_and(|a| a.trace);
        let h = {
            let mut x = Steps { sess: &mut *self, stream, limit_ms: 0 };
            age::hierarchy::read(&mut x).map_err(|r| Failure::client(&r.code, r.message))?
        };
        if trace {
            eprintln!("age-trace hierarchy {} labels, {} inheriting", h.vertex.len(), h.chains().len());
        }
        Ok(h)
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
        let counters = self.config.age.as_ref().is_none_or(|a| a.counters);
        if !st.schema.is_empty() {
            // the label tables and inheritance a multi-label CREATE / MERGE
            // needs, in this transaction, before it runs (`hierarchy.rs`)
            let mut x = Steps { sess: &mut *self, stream: &mut *stream, limit_ms };
            age::hierarchy::apply(&mut x, &st.schema).map_err(|r| Failure::client(&r.code, r.message))?;
        }
        match &st.plan {
            Plan::Cypher { text, ncols } => {
                let before = if st.write && counters { Some(self.age_counts()?) } else { None };
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
                let before = if st.write && counters { Some(self.age_counts()?) } else { None };
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
                let before = if counters { Some(self.age_counts()?) } else { None };
                let (rows, adjust) = {
                    let mut x = Steps { sess: &mut *self, stream: &mut *stream, limit_ms };
                    age::split::run(&mut x, steps, params).map_err(|r| Failure::client(&r.code, r.message))?
                };
                let stats = match before {
                    Some(b) => {
                        let mut stats = Stats::from_counts(self.age_counts()?.minus(b));
                        stats.add(&adjust);
                        Some(stats)
                    }
                    None => None,
                };
                let records = if steps.ncols == 0 {
                    Vec::new()
                } else {
                    rows.iter().map(|r| Value::List(r.iter().map(agtype::Ag::to_bolt).collect())).collect()
                };
                Ok(Outcome { records, stats: if st.write { stats } else { None } })
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

/// The UTC wall time of `$1` (a local date-time) in zone `$2`.
const ZONE_INSTANT: &str =
    "SELECT to_char(timezone('UTC', timezone($2::text, $1::text::timestamp)), 'YYYY-MM-DD\"T\"HH24:MI:SS.US')";

/// Does a parameter hold a DateTime with a zone id (Bolt 4.4 `0x66`)?
fn has_zone_id(v: &Value) -> bool {
    match v {
        Value::Struct(0x66, _) => true,
        Value::List(xs) => xs.iter().any(has_zone_id),
        Value::Map(m) => m.iter().any(|(_, v)| has_zone_id(v)),
        _ => false,
    }
}

/// The parameters as the AGE backend sends them: as `to_json` makes them for
/// the default backend, except a DateTime with a zone id.
///
/// SUITES-ON-AGE B9: `to_json` writes one as Neo4j prints it less the offset,
/// `2024-03-05T13:04:05[Asia/Seoul]` (PostgreSQL reads that as the local time
/// in the zone, which is what the default backend needs). Here nothing reads
/// the zone: `canonical_datetime` dropped it and took the wall time for UTC —
/// `datetime($zoned)` gave 13:04:05+00:00, nine hours off the instant
/// (04:04:05+00:00). So the gateway asks `zone` for the instant first, and
/// the parameter becomes that, in the canonical UTC form the other
/// date-times take here. A DateTime with an offset (`0x46`) is left as it is:
/// its offset says the instant already.
fn age_params(v: &Value, zone: &mut dyn FnMut(&str, &str) -> Result<String, Failure>) -> Result<serde_json::Value, Failure> {
    Ok(match v {
        Value::Struct(0x66, f) => {
            let int = |i: usize| {
                f.get(i).and_then(Value::as_i64).ok_or_else(|| Failure::client("Neo.ClientError.Statement.TypeError", "malformed DateTime structure".into()))
            };
            let name = f
                .get(2)
                .and_then(Value::as_str)
                .ok_or_else(|| Failure::client("Neo.ClientError.Statement.TypeError", "malformed DateTime structure".into()))?;
            serde_json::Value::String(zone(&super::date_time(int(0)?, int(1)?), name)?)
        }
        Value::List(xs) => serde_json::Value::Array(xs.iter().map(|x| age_params(x, zone)).collect::<Result<_, _>>()?),
        Value::Map(m) => serde_json::Value::Object(
            m.iter().map(|(k, x)| Ok((k.clone(), age_params(x, zone)?))).collect::<Result<_, Failure>>()?,
        ),
        other => super::to_json(other).map_err(|e| Failure::client("Neo.ClientError.Statement.TypeError", e))?,
    })
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
        if self.sess.config.age.as_ref().is_some_and(|a| a.trace) {
            eprintln!("age-trace > {sql}");
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn zoned(t: i64, nanos: i64, zone: &str) -> Value {
        Value::Struct(0x66, vec![Value::Int(t), Value::Int(nanos), Value::String(zone.into())])
    }

    #[test]
    fn a_zone_id_becomes_the_instant() {
        // 2024-03-05T13:04:05 (19787 days after the epoch), as the driver sends it
        let t = 19_787 * 86_400 + 3_600 * 13 + 60 * 4 + 5;
        let params = Value::Map(
            [
                ("zoned".to_string(), zoned(t, 0, "Asia/Seoul")),
                ("again".to_string(), zoned(t, 0, "Asia/Seoul")),
                ("rows".to_string(), Value::List(vec![Value::Map([("at".to_string(), zoned(t, 500_000_000, "Europe/Paris"))].into_iter().collect())])),
                ("offset".to_string(), Value::Struct(0x46, vec![Value::Int(t), Value::Int(0), Value::Int(9 * 3600)])),
                ("text".to_string(), Value::String("2024-03-05T13:04:05[Asia/Seoul]".into())),
            ]
            .into_iter()
            .collect(),
        );
        assert!(has_zone_id(&params));
        assert!(!has_zone_id(&Value::Map([("offset".to_string(), Value::Struct(0x46, vec![Value::Int(t), Value::Int(0), Value::Int(0)]))].into_iter().collect())));
        // what PostgreSQL's zone database would answer, as text in UTC
        let asked = std::cell::RefCell::new(Vec::new());
        let mut fake = |local: &str, zone: &str| -> Result<String, Failure> {
            asked.borrow_mut().push(format!("{local} {zone}"));
            let off = match zone {
                "Asia/Seoul" => "+09:00",
                "Europe/Paris" => "+01:00",
                _ => return Err(Failure::client("Neo.ClientError.Statement.TypeError", format!("time zone \"{zone}\" not recognized"))),
            };
            Ok(crate::age::cypher::canonical_datetime(&format!("{local}{off}")).unwrap())
        };
        let j = age_params(&params, &mut fake).ok().unwrap();
        assert_eq!(j["zoned"], serde_json::json!("2024-03-05T04:04:05+00:00"));
        assert_eq!(j["again"], serde_json::json!("2024-03-05T04:04:05+00:00"));
        assert_eq!(j["rows"][0]["at"], serde_json::json!("2024-03-05T12:04:05.5+00:00"));
        // an offset says the instant already, and a string stays a string
        assert_eq!(j["offset"], serde_json::json!("2024-03-05T13:04:05+09:00"));
        assert_eq!(j["text"], serde_json::json!("2024-03-05T13:04:05[Asia/Seoul]"));
        let a = asked.borrow().clone();
        assert!(a.contains(&"2024-03-05T13:04:05 Asia/Seoul".to_string()) && a.contains(&"2024-03-05T13:04:05.5 Europe/Paris".to_string()), "{a:?}");
        // a zone the database does not know is the client's error
        let bad = Value::Map([("z".to_string(), zoned(t, 0, "Mars/Olympus"))].into_iter().collect());
        let e = age_params(&bad, &mut fake).err().unwrap();
        assert_eq!(e.code, "Neo.ClientError.Statement.TypeError");
        // the default backend's text is unchanged
        assert_eq!(super::super::to_json(&params).unwrap()["zoned"], serde_json::json!("2024-03-05T13:04:05[Asia/Seoul]"));
    }
}
