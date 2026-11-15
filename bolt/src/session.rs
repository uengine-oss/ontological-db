//! One Bolt connection: handshake, message loop, and the mapping between
//! `og_cypher()` results and Bolt values (spec 011, FR-005…FR-017).

use crate::packstream::{self as ps, map, string, Value};
use crate::Config;
use postgres::error::SqlState;
use postgres::{CancelToken, Client, NoTls};
use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const MAGIC: [u8; 4] = [0x60, 0x60, 0xB0, 0x17];
/// Bolt 4.4, encoded as the protocol does it: major in the low byte.
const BOLT_4_4: u32 = 0x0000_0404;
const NO_VERSION: u32 = 0x0000_0000;

// client → server
const HELLO: u8 = 0x01;
const GOODBYE: u8 = 0x02;
const RESET: u8 = 0x0F;
const RUN: u8 = 0x10;
const BEGIN: u8 = 0x11;
const COMMIT: u8 = 0x12;
const ROLLBACK: u8 = 0x13;
const DISCARD: u8 = 0x2F;
const PULL: u8 = 0x3F;
const ROUTE: u8 = 0x66;

// server → client
const SUCCESS: u8 = 0x70;
const RECORD: u8 = 0x71;
const IGNORED: u8 = 0x7E;
const FAILURE: u8 = 0x7F;

// graph structures
const NODE: u8 = 0x4E;
const RELATIONSHIP: u8 = 0x52;

#[derive(PartialEq)]
enum State {
    /// Ready for a query.
    Ready,
    /// A result is open; only PULL/DISCARD/RESET make sense.
    Streaming,
    /// Something failed. Everything is IGNORED until RESET (FR-007).
    Failed,
}

struct Session<'a> {
    config: &'a Config,
    pg: Option<Client>,
    state: State,
    graph: String,
    in_tx: bool,
    /// Rows fetched but not yet PULLed, and the fields they belong to.
    pending: Vec<Value>,
    fields: Vec<String>,
    cursor: usize,
    /// What the open result is: `"r"` or `"w"`, reported as the summary's
    /// `type`. Drivers expose it as `ResultSummary.query_type`, and tools built
    /// on the driver read it to decide whether a statement is allowed to run.
    qtype: &'static str,
    /// Settings from the open transaction's `tx_metadata` (spec 014 FR-010,
    /// spec 013 FR-009), re-applied with every RUN inside it.
    tx_meta: Vec<(String, String)>,
    /// Bytes the client sent while a query ran. They were read so that a
    /// closed connection could be seen (see [`Watch`]); the message loop
    /// consumes them before reading the socket again.
    early: VecDeque<u8>,
    /// The session's statement limit in milliseconds, as the engine reports it
    /// (`og_statement_timeout()`); 0 when there is none or it is unknown.
    timeout_ms: i64,
    /// The open transaction's own limit, from BEGIN's `tx_timeout`.
    tx_timeout_ms: Option<i64>,
}

/// The `tx_metadata` keys the gateway understands, and the setting each one
/// becomes. Neo4j drivers already carry application metadata this way, so a
/// client names who it is acting for — or the instant it wants to read at —
/// without a new API. Unknown keys are ignored.
const TX_SETTINGS: &[(&str, &str)] = &[
    ("actor", "og.actor"),
    ("source", "og.source"),
    ("reason", "og.reason"),
    ("request_id", "og.request_id"),
    ("requestId", "og.request_id"),
    ("valid_at", "og.valid_at"),
    ("validAt", "og.valid_at"),
    ("system_at", "og.system_at"),
    ("systemAt", "og.system_at"),
    ("valid_from", "og.valid_from"),
    ("validFrom", "og.valid_from"),
    ("valid_to", "og.valid_to"),
    ("validTo", "og.valid_to"),
];

/// A driver's transaction timeout — `extra.tx_timeout`, in milliseconds, which
/// Neo4j drivers send for `Query(…, timeout=…)` and `begin_transaction(timeout=…)`.
/// It becomes the engine's `og.statement_timeout` for that transaction; 0 means
/// no limit, as it does for Neo4j.
fn tx_timeout(extra: &Value) -> Option<i64> {
    extra.map_get("tx_timeout").and_then(Value::as_i64).filter(|ms| *ms >= 0)
}

/// `extra.tx_metadata` → (setting, value) pairs.
fn tx_settings(extra: &Value) -> Vec<(String, String)> {
    let Some(Value::Map(meta)) = extra.map_get("tx_metadata") else { return Vec::new() };
    let mut out = Vec::new();
    for (key, setting) in TX_SETTINGS {
        let Some(v) = meta.get(*key) else { continue };
        let text = match v {
            Value::String(s) => s.clone(),
            Value::Null => continue,
            other => match to_json(other) {
                serde_json::Value::String(s) => s,
                j => j.to_string(),
            },
        };
        if !out.iter().any(|(s, _): &(String, String)| s == setting) {
            out.push((setting.to_string(), text));
        }
    }
    out
}

pub fn serve(mut stream: TcpStream, config: &Config) -> io::Result<()> {
    if !handshake(&mut stream)? {
        return Ok(());
    }
    stream.set_nodelay(true).ok();
    let mut sess = Session {
        config,
        pg: None,
        state: State::Ready,
        graph: config.default_graph.clone(),
        in_tx: false,
        pending: Vec::new(),
        fields: Vec::new(),
        cursor: 0,
        qtype: "r",
        tx_meta: Vec::new(),
        early: VecDeque::new(),
        timeout_ms: 0,
        tx_timeout_ms: None,
    };
    sess.run(&mut stream)
}

/// The message loop's view of the connection: bytes read ahead first, then
/// the socket.
struct Incoming<'a> {
    early: &'a mut VecDeque<u8>,
    stream: &'a mut TcpStream,
}

impl Read for Incoming<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.early.is_empty() {
            return self.stream.read(buf);
        }
        let n = buf.len().min(self.early.len());
        for (slot, byte) in buf.iter_mut().zip(self.early.drain(..n)) {
            *slot = byte;
        }
        Ok(n)
    }
}

/// Magic preamble plus four proposed versions; we answer with the one we speak
/// or with zero, which drivers report as a clean negotiation failure (FR-001).
fn handshake(stream: &mut TcpStream) -> io::Result<bool> {
    let mut preamble = [0u8; 20];
    stream.read_exact(&mut preamble)?;
    if preamble[..4] != MAGIC {
        return Ok(false);
    }
    let agreed = preamble[4..]
        .chunks(4)
        .map(|c| u32::from_be_bytes(c.try_into().unwrap()))
        .find(|proposal| speaks(*proposal))
        .map(|_| BOLT_4_4)
        .unwrap_or(NO_VERSION);
    stream.write_all(&agreed.to_be_bytes())?;
    stream.flush()?;
    Ok(agreed != NO_VERSION)
}

/// A proposal may carry a range in its third byte: "4.4 down to 4.0".
fn speaks(proposal: u32) -> bool {
    let major = proposal & 0xFF;
    let minor = (proposal >> 8) & 0xFF;
    let range = (proposal >> 16) & 0xFF;
    major == 4 && minor >= 4 && minor - range.min(minor) <= 4
}

impl Session<'_> {
    fn run(&mut self, stream: &mut TcpStream) -> io::Result<()> {
        loop {
            let msg = ps::read_message(&mut Incoming { early: &mut self.early, stream: &mut *stream })?;
            let Value::Struct(sig, fields) = msg else {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "bolt: not a message"));
            };
            if sig == GOODBYE {
                return Ok(());
            }
            if self.state == State::Failed && sig != RESET {
                ps::write_message(stream, &Value::Struct(IGNORED, vec![]))?;
                continue;
            }
            let reply = self.dispatch(sig, fields, stream)?;
            if let Some(reply) = reply {
                ps::write_message(stream, &reply)?;
            }
        }
    }

    /// Returns the message to send back, or None when `dispatch` already wrote
    /// its own (records, then a summary).
    fn dispatch(
        &mut self,
        sig: u8,
        mut fields: Vec<Value>,
        stream: &mut TcpStream,
    ) -> io::Result<Option<Value>> {
        let result = match sig {
            HELLO => self.hello(fields.pop().unwrap_or(Value::Null)),
            RUN => self.run_query(fields, stream),
            PULL => return self.pull(fields.pop().unwrap_or(Value::Null), stream, true),
            DISCARD => return self.pull(fields.pop().unwrap_or(Value::Null), stream, false),
            BEGIN => self.begin(fields.pop().unwrap_or(Value::Null)),
            COMMIT => self.end_tx("COMMIT"),
            ROLLBACK => self.end_tx("ROLLBACK"),
            RESET => self.reset(),
            ROUTE => Ok(self.routing_table()),
            other => Err(Failure::client(
                "Neo.ClientError.Request.Invalid",
                format!("unsupported Bolt message 0x{other:02X}"),
            )),
        };
        Ok(Some(match result {
            Ok(meta) => Value::Struct(SUCCESS, vec![meta]),
            Err(f) => {
                self.state = State::Failed;
                f.into_message()
            }
        }))
    }

    // ------------------------------------------------------------- session

    /// Authentication is PostgreSQL's: the credentials in HELLO are the role's
    /// credentials, and failing to connect is what "unauthorized" means here.
    /// No second user store (FR-015).
    fn hello(&mut self, extra: Value) -> Result<Value, Failure> {
        let user = extra.map_get("principal").and_then(Value::as_str).unwrap_or("").to_string();
        let password =
            extra.map_get("credentials").and_then(Value::as_str).unwrap_or("").to_string();

        let mut cfg = postgres::Config::new();
        cfg.host(&self.config.pg_host)
            .port(self.config.pg_port)
            .dbname(&self.config.pg_database)
            .user(&user)
            .application_name("ontological-bolt");
        if !password.is_empty() {
            cfg.password(&password);
        }
        let mut client = cfg.connect(NoTls).map_err(|e| {
            Failure::client("Neo.ClientError.Security.Unauthorized", pg_message(&e))
        })?;
        // If this process dies mid-query its PostgreSQL connection closes, but a
        // busy backend only finds out when it next writes — after the query, which
        // may be never. With a check interval it looks while it works. Windows
        // has no way to ask, and refuses the setting; that is not worth failing
        // a login over.
        let _ = client.batch_execute("SET client_connection_check_interval = '2s'");
        if let Some(limit) = &self.config.statement_timeout {
            client
                .execute("SELECT set_config('og.statement_timeout', $1, false)", &[limit])
                .map_err(|e| Failure::from_pg(&e))?;
        }
        // What the limit is, to say so when it is hit and to stand behind it
        // (see `Watch`). An engine without the function simply has none here.
        self.timeout_ms = client
            .query_one("SELECT og_statement_timeout()", &[])
            .map(|r| r.get::<_, i64>(0))
            .unwrap_or(0);
        self.pg = Some(client);

        Ok(map(vec![
            // Drivers gate features on this string; we are Bolt 4.4, so we say so.
            ("server", string("Neo4j/4.4.0 (ontological-bolt)")),
            ("connection_id", string(format!("bolt-{}", std::process::id()))),
            ("hints", Value::Map(BTreeMap::new())),
        ]))
    }

    fn reset(&mut self) -> Result<Value, Failure> {
        self.pending.clear();
        self.cursor = 0;
        if self.in_tx {
            if let Some(pg) = self.pg.as_mut() {
                let _ = pg.batch_execute("ROLLBACK");
            }
            self.in_tx = false;
        }
        self.tx_meta.clear();
        self.tx_timeout_ms = None;
        self.state = State::Ready;
        Ok(Value::Map(BTreeMap::new()))
    }

    fn begin(&mut self, extra: Value) -> Result<Value, Failure> {
        self.select_graph(&extra);
        self.sql("BEGIN")?;
        self.in_tx = true;
        self.tx_meta = tx_settings(&extra);
        self.tx_timeout_ms = tx_timeout(&extra);
        if let Some(ms) = self.tx_timeout_ms {
            self.tx_meta.push(("og.statement_timeout".into(), ms.to_string()));
        }
        let meta = self.tx_meta.clone();
        self.apply_settings(&meta)?;
        Ok(Value::Map(BTreeMap::new()))
    }

    /// Transaction-local, so nothing outlives the transaction it was sent with.
    fn apply_settings(&mut self, settings: &[(String, String)]) -> Result<(), Failure> {
        for (k, v) in settings {
            let pg = self.client()?;
            pg.execute("SELECT set_config($1, $2, true)", &[k, v])
                .map_err(|e| Failure::from_pg(&e))?;
        }
        Ok(())
    }

    fn end_tx(&mut self, verb: &str) -> Result<Value, Failure> {
        self.sql(verb)?;
        self.in_tx = false;
        self.tx_meta.clear();
        self.tx_timeout_ms = None;
        Ok(map(vec![("bookmark", string("ontological:0"))]))
    }

    /// A session's "database" is a graph here (FR-016).
    fn select_graph(&mut self, extra: &Value) {
        if let Some(db) = extra.map_get("db").and_then(Value::as_str) {
            // "neo4j" and "system" are what drivers send when the application
            // named nothing; they mean "the default", not a graph called neo4j.
            if !db.is_empty() && db != "neo4j" && db != "system" {
                self.graph = db.to_string();
                return;
            }
        }
        // Inside an explicit transaction the driver names the database once, on
        // BEGIN, and omits it from every RUN that follows — so an absent `db`
        // means "the one this transaction began with", not "the default".
        if self.in_tx {
            return;
        }
        self.graph = self.config.default_graph.clone();
    }

    // --------------------------------------------------------------- query

    fn run_query(&mut self, mut fields: Vec<Value>, stream: &mut TcpStream) -> Result<Value, Failure> {
        let extra = if fields.len() > 2 { fields.remove(2) } else { Value::Null };
        let params = if fields.len() > 1 { fields.remove(1) } else { Value::Null };
        let query = fields
            .first()
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Failure::client("Neo.ClientError.Request.Invalid", "RUN without a query".into())
            })?
            .to_string();
        self.select_graph(&extra);

        let params_json = to_json(&params).to_string();

        // `EXPLAIN` / `PROFILE` prefix a query to ask what it *would* do. Neo4j
        // answers with a plan and no records; the one field of that answer that
        // clients act on is the summary's `type`, which is how a driver-side
        // tool tells a read from a write before it runs anything. The official
        // Neo4j MCP server gates both of its query tools on exactly that.
        //
        // The gateway still parses nothing: it asks `og_cypher_check()`, so the
        // read/write verdict is spec 003's, reached through a second transport
        // rather than reimplemented behind one.
        let (body, plan_only) = split_plan_prefix(&query);
        let write = self.is_write(body)?;
        self.qtype = if write { "w" } else { "r" };

        if plan_only {
            self.fields = Vec::new();
            self.pending = Vec::new();
            self.cursor = 0;
            self.state = State::Streaming;
            return Ok(map(vec![
                ("fields", Value::List(Vec::new())),
                ("t_first", Value::Int(0)),
                ("qid", Value::Int(-1)),
            ]));
        }

        // Field order comes from the parser, not from the row: jsonb sorts its
        // keys, so the row cannot tell us what the query asked for (FR-010).
        let columns: Vec<String> = {
            let pg = self.client()?;
            let row = pg
                .query_one("SELECT og_cypher_columns($1::text)", &[&query])
                .map_err(|e| Failure::from_pg(&e))?;
            row.get(0)
        };

        // An auto-commit RUN may carry its own tx_metadata and timeout. Settings
        // are transaction-local, so the query gets a transaction to hold them.
        let (own, limit_ms) = if self.in_tx {
            (Vec::new(), self.tx_timeout_ms.unwrap_or(self.timeout_ms))
        } else {
            let mut own = tx_settings(&extra);
            let limit = tx_timeout(&extra);
            if let Some(ms) = limit {
                own.push(("og.statement_timeout".into(), ms.to_string()));
            }
            (own, limit.unwrap_or(self.timeout_ms))
        };
        let wrap = !own.is_empty();
        if wrap {
            self.sql("BEGIN")?;
            if let Err(f) = self.apply_settings(&own) {
                let _ = self.sql("ROLLBACK");
                return Err(f);
            }
        }
        let rows = {
            let graph = self.graph.clone();
            let pg = self.client()?;
            // While PostgreSQL works this thread is blocked, so another one
            // watches the client: a closed connection or a RESET cancels the
            // query, which otherwise ran on with nobody left to read it.
            let watch = Watch::start(stream, pg.cancel_token(), limit_ms);
            let result = pg.query(
                "SELECT og_cypher($1::text, $2::text, $3::text::jsonb)::text",
                &[&graph, &query, &params_json],
            );
            let seen = watch.finish(stream);
            self.early.extend(seen.bytes);
            result.map_err(|e| Failure::from_query(&e, seen.interrupt, limit_ms))
        };
        let rows = match (rows, wrap) {
            (Ok(r), true) => {
                self.sql("COMMIT")?;
                r
            }
            (Ok(r), false) => r,
            (Err(f), true) => {
                let _ = self.sql("ROLLBACK");
                return Err(f);
            }
            (Err(f), false) => return Err(f),
        };

        let mut records = Vec::with_capacity(rows.len());
        let mut fields_out = columns;
        for row in &rows {
            let raw: Option<String> = row.get(0);
            let json: serde_json::Value = match raw {
                Some(t) => serde_json::from_str(&t).unwrap_or(serde_json::Value::Null),
                None => serde_json::Value::Null,
            };
            if fields_out.is_empty() {
                // `RETURN *` and other cases the parser cannot order: fall back
                // to the row's own keys rather than inventing an order.
                if let serde_json::Value::Object(m) = &json {
                    fields_out = m.keys().cloned().collect();
                }
            }
            records.push(record(&json, &fields_out));
        }

        self.fields = fields_out.clone();
        self.pending = records;
        self.cursor = 0;
        self.state = State::Streaming;

        Ok(map(vec![
            ("fields", Value::List(fields_out.into_iter().map(Value::String).collect())),
            ("t_first", Value::Int(0)),
            ("qid", Value::Int(-1)),
        ]))
    }

    /// PULL streams up to `n` records then summarises; DISCARD drops them and
    /// summarises. `n = -1` means "all" (FR-012).
    fn pull(
        &mut self,
        extra: Value,
        stream: &mut TcpStream,
        deliver: bool,
    ) -> io::Result<Option<Value>> {
        if self.state != State::Streaming {
            let f = Failure::client(
                "Neo.ClientError.Request.Invalid",
                "no result is open on this connection".into(),
            );
            self.state = State::Failed;
            return Ok(Some(f.into_message()));
        }
        let want = extra.map_get("n").and_then(Value::as_i64).unwrap_or(-1);
        let remaining = self.pending.len() - self.cursor;
        let take = if want < 0 { remaining } else { (want as usize).min(remaining) };

        if deliver {
            for i in 0..take {
                let rec = self.pending[self.cursor + i].clone();
                ps::write_message(stream, &Value::Struct(RECORD, vec![rec]))?;
            }
        }
        self.cursor += take;

        let has_more = self.cursor < self.pending.len();
        let mut meta = vec![("t_last", Value::Int(0))];
        if has_more {
            meta.push(("has_more", Value::Bool(true)));
        } else {
            self.state = State::Ready;
            self.pending.clear();
            self.cursor = 0;
            meta.push(("type", string(self.qtype)));
            // A driver exposes this as `ResultSummary.counters`. It is asked for
            // only after a write, on the same connection that performed it —
            // `og_cypher_stats()` reports what the previous call on this
            // backend changed, and there is no other window in which that is
            // the right answer.
            if self.qtype == "w" {
                if let Some(stats) = self.write_stats() {
                    meta.push(("stats", stats));
                }
            }
            meta.push(("db", string(self.graph.clone())));
            if !self.in_tx {
                meta.push(("bookmark", string("ontological:0")));
            }
        }
        Ok(Some(Value::Struct(SUCCESS, vec![map(meta)])))
    }

    /// One server, announced as every role. A real routing table belongs to the
    /// cluster spec (007); this exists so `neo4j://` URIs connect at all.
    fn routing_table(&self) -> Value {
        let addr = string(self.config.advertised.clone());
        let servers = ["WRITE", "READ", "ROUTE"]
            .iter()
            .map(|role| {
                map(vec![
                    ("addresses", Value::List(vec![addr.clone()])),
                    ("role", string(*role)),
                ])
            })
            .collect();
        map(vec![(
            "rt",
            map(vec![
                ("ttl", Value::Int(300)),
                ("db", string(self.graph.clone())),
                ("servers", Value::List(servers)),
            ]),
        )])
    }

    // ------------------------------------------------------------ plumbing

    fn client(&mut self) -> Result<&mut Client, Failure> {
        self.pg.as_mut().ok_or_else(|| {
            Failure::client(
                "Neo.ClientError.Security.Unauthorized",
                "no HELLO on this connection".into(),
            )
        })
    }

    fn sql(&mut self, stmt: &str) -> Result<(), Failure> {
        let pg = self.client()?;
        pg.batch_execute(stmt).map_err(|e| Failure::from_pg(&e))
    }

    /// The change counts of the write that just ran, as a Bolt map.
    ///
    /// Best effort on purpose: a summary is not worth failing a successful
    /// write over, so a connection that cannot answer simply reports no
    /// counters — which is what it did before there were any.
    fn write_stats(&mut self) -> Option<Value> {
        let pg = self.pg.as_mut()?;
        let row = pg.query_one("SELECT og_cypher_stats()::text", &[]).ok()?;
        let raw: String = row.get(0);
        let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
        Some(to_bolt(&parsed))
    }

    /// Does this query write? Answered by the engine's own parser, never by a
    /// keyword scan here — `CREATE` inside a string literal is not a write, and
    /// only the parser knows that.
    ///
    /// A query that does not parse gets its syntax error now rather than after
    /// `EXPLAIN` has silently reported it as a read.
    fn is_write(&mut self, query: &str) -> Result<bool, Failure> {
        let pg = self.client()?;
        let row = pg
            .query_one("SELECT og_cypher_check($1::text)::text", &[&query])
            .map_err(|e| Failure::from_pg(&e))?;
        let raw: String = row.get(0);
        let check: serde_json::Value =
            serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
        if check.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let msg = check
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("query did not parse")
                .to_string();
            return Err(Failure::client("Neo.ClientError.Statement.SyntaxError", msg));
        }
        Ok(check.get("write").and_then(|v| v.as_bool()).unwrap_or(false))
    }
}

/// Strip a leading `EXPLAIN` or `PROFILE`, returning the query underneath and
/// whether one was there.
///
/// Both are planning prefixes in Cypher, not clauses: what follows is an
/// ordinary query. `PROFILE` additionally means "execute and instrument"; there
/// are no plan statistics to instrument here, so it is treated as `EXPLAIN` —
/// the summary is honest about the query type and carries no invented plan.
fn split_plan_prefix(query: &str) -> (&str, bool) {
    let trimmed = query.trim_start();
    for kw in ["explain", "profile"] {
        if starts_with_keyword(trimmed, kw) {
            // The keyword and the whitespace after it are ASCII, so this is a
            // character boundary.
            let rest = trimmed[kw.len()..].trim_start();
            // `EXPLAIN FABRIC <query>` is a statement of its own — it runs the
            // query and reports what each source was sent — not a request for
            // a plan with no records.
            if starts_with_keyword(rest, "fabric") {
                return (query, false);
            }
            return (rest, true);
        }
    }
    (query, false)
}

/// Whether `s` begins with the ASCII keyword `kw` (any case) followed by
/// whitespace. Compared as bytes: a query whose first bytes hold a multibyte
/// character (`// 소스 …`, `WITH '🙂' …`) is simply not a match — slicing the
/// `str` at `kw.len()` there would not be on a character boundary and panics
/// the session thread.
fn starts_with_keyword(s: &str, kw: &str) -> bool {
    let b = s.as_bytes();
    b.len() > kw.len() && b[..kw.len()].eq_ignore_ascii_case(kw.as_bytes()) && b[kw.len()].is_ascii_whitespace()
}

// ------------------------------------------------------------------ values

/// One row, projected into the field order the query asked for.
fn record(json: &serde_json::Value, fields: &[String]) -> Value {
    let values = fields
        .iter()
        .map(|f| json.get(f).map(to_bolt).unwrap_or(Value::Null))
        .collect();
    Value::List(values)
}

/// `og_cypher()` describes a node as `{_id, _type, …props}` and a relationship
/// as `{_id, _type, _src, _dst, …props}`. Drivers want those as structures, so
/// that `Node` and `Relationship` objects come out the other end (FR-011).
fn to_bolt(v: &serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => Value::Int(i),
            None => Value::Float(n.as_f64().unwrap_or(f64::NAN)),
        },
        serde_json::Value::String(s) => Value::String(s.clone()),
        serde_json::Value::Array(xs) => Value::List(xs.iter().map(to_bolt).collect()),
        serde_json::Value::Object(m) => {
            let id = m.get("_id").and_then(serde_json::Value::as_i64);
            let ty = m.get("_type").and_then(serde_json::Value::as_str);
            let (src, dst) = (
                m.get("_src").and_then(serde_json::Value::as_i64),
                m.get("_dst").and_then(serde_json::Value::as_i64),
            );
            let props = || {
                Value::Map(
                    m.iter()
                        .filter(|(k, _)| !k.starts_with('_'))
                        .map(|(k, v)| (k.clone(), to_bolt(v)))
                        .collect(),
                )
            };
            match (id, ty, src, dst) {
                (Some(id), Some(ty), Some(src), Some(dst)) => Value::Struct(
                    RELATIONSHIP,
                    vec![
                        Value::Int(id),
                        Value::Int(src),
                        Value::Int(dst),
                        string(ty),
                        props(),
                    ],
                ),
                (Some(id), Some(ty), _, _) => Value::Struct(
                    NODE,
                    vec![Value::Int(id), Value::List(vec![string(ty)]), props()],
                ),
                _ => Value::Map(m.iter().map(|(k, v)| (k.clone(), to_bolt(v))).collect()),
            }
        }
    }
}

/// Parameters travel to `og_cypher()` as jsonb, never interpolated into the
/// query text — the injection guarantee is spec 003's FR-026, unchanged here.
fn to_json(v: &Value) -> serde_json::Value {
    match v {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Int(i) => serde_json::Value::from(*i),
        Value::Float(f) => serde_json::Value::from(*f),
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::List(xs) => serde_json::Value::Array(xs.iter().map(to_json).collect()),
        Value::Map(m) => {
            serde_json::Value::Object(m.iter().map(|(k, v)| (k.clone(), to_json(v))).collect())
        }
        // Drivers can send temporal/spatial structures; 4.4 support for those is
        // out of scope (spec 011), and silently mangling them would be worse.
        Value::Struct(sig, _) => serde_json::Value::String(format!("<unsupported struct 0x{sig:02X}>")),
    }
}

// ------------------------------------------------------------ watching

/// Why a running query was cancelled from this side.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Interrupt {
    /// The client closed the connection, or it broke.
    ClientGone,
    /// The client sent RESET: Bolt's way of interrupting what is running.
    Reset,
    /// The engine's own limit should have ended it by now and has not.
    Deadline,
}

/// What a watch saw: the bytes it read, and whether it cancelled the query.
struct Seen {
    bytes: Vec<u8>,
    interrupt: Option<Interrupt>,
}

/// A second thread that reads the client while the session thread waits on
/// PostgreSQL.
///
/// Before this, a query outlived everyone who wanted it. The session thread
/// sat in `query()` and read nothing, so a client that gave up — a driver
/// timeout, Ctrl-C, a closed tab — went unnoticed; the gateway never asked
/// PostgreSQL to stop, and PostgreSQL, whose client (this gateway) was still
/// connected, had no reason to. A cartesian product over two source tables
/// ran until the source finished.
///
/// Reading is the only way to see a close: the driver has usually pipelined
/// PULL behind RUN, so the end of the stream sits behind bytes that must be
/// consumed first. They are kept and handed back to the message loop.
///
/// The deadline is a backstop. The engine cancels at `og.statement_timeout`
/// itself; this cancels a few seconds later only if that did not happen.
struct Watch {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<Seen>>,
}

/// How long past the engine's limit the gateway waits before cancelling itself.
const DEADLINE_GRACE: Duration = Duration::from_secs(5);
/// How often the watcher looks at the socket and the clock. It waits by
/// parking, and `finish` unparks it: a statement's end never waits out a tick.
/// (It used to sleep, and `finish` joined the sleeping thread — every
/// statement over Bolt cost up to one tick, ~100 ms, even `RETURN 1`.)
const WATCH_TICK: Duration = Duration::from_millis(100);

impl Watch {
    fn start(stream: &TcpStream, token: CancelToken, limit_ms: i64) -> Watch {
        let stop = Arc::new(AtomicBool::new(false));
        // Non-blocking rather than a read timeout: on Windows a timed-out
        // receive leaves the socket in an undefined state. The session thread
        // is blocked in PostgreSQL and does not touch the socket meanwhile.
        let sock = match stream.try_clone() {
            Ok(s) if s.set_nonblocking(true).is_ok() => s,
            // No second handle on the socket: run unwatched, as before.
            _ => return Watch { stop, handle: None },
        };
        let deadline = (limit_ms > 0)
            .then(|| Instant::now() + Duration::from_millis(limit_ms as u64) + DEADLINE_GRACE);
        let flag = stop.clone();
        let handle = thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut buf = [0u8; 4096];
            let mut interrupt = None;
            while !flag.load(Ordering::SeqCst) {
                match (&sock).read(&mut buf) {
                    Ok(0) => interrupt = Some(Interrupt::ClientGone),
                    Ok(n) => {
                        bytes.extend_from_slice(&buf[..n]);
                        if holds_reset(&bytes) {
                            interrupt = Some(Interrupt::Reset);
                        }
                    }
                    // Parked, not asleep: `finish` sets the flag and unparks,
                    // and the loop ends at once (an unpark that comes first
                    // makes this return immediately).
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => thread::park_timeout(WATCH_TICK),
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => interrupt = Some(Interrupt::ClientGone),
                }
                if interrupt.is_none() && deadline.is_some_and(|d| Instant::now() >= d) {
                    interrupt = Some(Interrupt::Deadline);
                }
                if interrupt.is_some() {
                    // The query may have finished a moment ago; a cancel request
                    // that finds the backend idle is ignored.
                    if !flag.load(Ordering::SeqCst) {
                        let _ = token.cancel_query(NoTls);
                    }
                    break;
                }
            }
            Seen { bytes, interrupt }
        });
        Watch { stop, handle: Some(handle) }
    }

    /// Stop watching and take what was read. Restores blocking reads.
    fn finish(mut self, stream: &TcpStream) -> Seen {
        let watched = self.handle.is_some();
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = &self.handle {
            h.thread().unpark();
        }
        let seen = self
            .handle
            .take()
            .and_then(|h| h.join().ok())
            .unwrap_or(Seen { bytes: Vec::new(), interrupt: None });
        // The mode was set through the clone, but it belongs to the socket.
        if watched {
            let _ = stream.set_nonblocking(false);
        }
        seen
    }
}

/// Does this run of Bolt chunks hold a complete RESET message?
fn holds_reset(buf: &[u8]) -> bool {
    let mut i = 0;
    let mut message_start = true;
    while i + 2 <= buf.len() {
        let n = u16::from_be_bytes([buf[i], buf[i + 1]]) as usize;
        i += 2;
        if n == 0 {
            message_start = true;
            continue;
        }
        if i + n > buf.len() {
            return false;
        }
        // A message is one structure: its marker (0xB0 | field count), then
        // its signature.
        if message_start && n >= 2 && buf[i] & 0xF0 == 0xB0 && buf[i + 1] == RESET {
            return true;
        }
        message_start = false;
        i += n;
    }
    false
}

// ----------------------------------------------------------------- failures

struct Failure {
    code: String,
    message: String,
}

impl Failure {
    fn client(code: &str, message: String) -> Self {
        Failure { code: code.to_string(), message }
    }

    /// A query that did not finish because it was cancelled says why, under the
    /// code a Neo4j driver expects for that reason: over its time limit, or
    /// stopped (a RESET, or someone cancelling the backend). Anything else is
    /// [`Failure::from_pg`].
    fn from_query(e: &postgres::Error, interrupt: Option<Interrupt>, limit_ms: i64) -> Self {
        if e.code() != Some(&SqlState::QUERY_CANCELED) {
            return Failure::from_pg(e);
        }
        let message = pg_message(e);
        let timed_out = message.contains("statement timeout") || interrupt == Some(Interrupt::Deadline);
        if timed_out {
            let limit = if limit_ms > 0 {
                format!("its time limit ({} s, og.statement_timeout)", (limit_ms + 999) / 1000)
            } else {
                "its time limit (og.statement_timeout)".to_string()
            };
            return Failure::client(
                "Neo.ClientError.Transaction.TransactionTimedOut",
                format!(
                    "the query ran past {limit} and was cancelled, here and at the source. \
                     Narrow it — label every node, connect the patterns through relationships \
                     (comma-separated patterns are a cartesian product), filter early, add LIMIT — \
                     or give this transaction a longer timeout. ({message})"
                ),
            );
        }
        Failure::client("Neo.TransientError.Transaction.Terminated", message)
    }

    /// The compiler's message is the valuable part — it names the construct and
    /// suggests the alternative (spec 003 FR-008), and an agent retries on it
    /// (principle VIII). It is passed through verbatim; only the code is ours.
    fn from_pg(e: &postgres::Error) -> Self {
        let message = pg_message(e);
        let code = error_code(e.code(), &message);
        Failure::client(code, message)
    }

    fn into_message(self) -> Value {
        Value::Struct(
            FAILURE,
            vec![map(vec![("code", string(self.code)), ("message", string(self.message))])],
        )
    }
}

/// The Neo4j status code for a failed statement.
///
/// The SQLSTATE decides where PostgreSQL gives one that means something to a
/// Cypher client. The message used to decide alone, and "does not exist" read
/// as a missing database — so `function sum(jsonb) does not exist` and
/// `operator does not exist: boolean = integer` reached drivers as
/// `DatabaseNotFound`, which says to look for a different database rather than
/// to fix the query. Only a missing database or graph is that now.
fn error_code(state: Option<&SqlState>, message: &str) -> &'static str {
    if let Some(state) = state {
        let by_state = match state.code() {
            // invalid_catalog_name: the database itself
            "3D000" => Some("Neo.ClientError.Database.DatabaseNotFound"),
            "42501" => Some("Neo.ClientError.Security.Forbidden"),
            // no function/operator for these types, mismatched types, a value
            // that does not read as its type, an ambiguous call
            "42883" | "42804" | "42846" | "42725" | "22P02" | "2202E" => {
                Some("Neo.ClientError.Statement.TypeError")
            }
            "22012" | "22003" => Some("Neo.ClientError.Statement.ArithmeticError"),
            // the compiled SQL names a relation or column that is not there:
            // the engine's state, not the query's spelling
            "42P01" | "42703" => Some("Neo.DatabaseError.Statement.ExecutionFailed"),
            _ => None,
        };
        if let Some(code) = by_state {
            return code;
        }
    }
    // The engine's own errors (XX000) carry no telling SQLSTATE; read them.
    if message.contains("does not exist")
        && (message.contains("graph '") || message.contains("database \""))
    {
        "Neo.ClientError.Database.DatabaseNotFound"
    } else if message.contains("not supported")
        || message.contains("expected")
        || message.contains("unknown label")
        || message.contains("unknown function")
        || message.contains("is not defined")
    {
        "Neo.ClientError.Statement.SyntaxError"
    } else if message.contains("permission denied") {
        "Neo.ClientError.Security.Forbidden"
    } else if message.contains("does not exist") {
        "Neo.ClientError.Statement.SemanticError"
    } else {
        "Neo.ClientError.Statement.ArgumentError"
    }
}


fn pg_message(e: &postgres::Error) -> String {
    e.as_db_error().map(|d| d.message().to_string()).unwrap_or_else(|| e.to_string())
}

#[cfg(test)]
mod error_code_tests {
    use super::*;

    #[test]
    fn type_errors_are_not_a_missing_database() {
        let undefined = SqlState::from_code("42883");
        assert_eq!(
            error_code(Some(&undefined), "function sum(jsonb) does not exist"),
            "Neo.ClientError.Statement.TypeError"
        );
        assert_eq!(
            error_code(Some(&undefined), "operator does not exist: boolean = integer"),
            "Neo.ClientError.Statement.TypeError"
        );
        assert_eq!(
            error_code(Some(&SqlState::from_code("22P02")), "invalid input syntax for type numeric: \"x\""),
            "Neo.ClientError.Statement.TypeError"
        );
        assert_eq!(
            error_code(Some(&SqlState::from_code("22012")), "division by zero"),
            "Neo.ClientError.Statement.ArithmeticError"
        );
    }

    #[test]
    fn a_missing_graph_or_database_still_is() {
        let internal = SqlState::from_code("XX000");
        assert_eq!(
            error_code(Some(&internal), "graph 'nope' does not exist"),
            "Neo.ClientError.Database.DatabaseNotFound"
        );
        assert_eq!(
            error_code(Some(&SqlState::from_code("3D000")), "database \"x\" does not exist"),
            "Neo.ClientError.Database.DatabaseNotFound"
        );
        // Anything else that "does not exist" is about the query.
        assert_eq!(
            error_code(Some(&internal), "type 'Nope' does not exist in this graph"),
            "Neo.ClientError.Statement.SemanticError"
        );
        assert_eq!(
            error_code(Some(&internal), "cypher error: unknown function 'nope'. supported: …"),
            "Neo.ClientError.Statement.SyntaxError"
        );
    }
}

#[cfg(test)]
mod tx_metadata_tests {
    use super::*;

    #[test]
    fn known_keys_become_settings() {
        let extra = map(vec![(
            "tx_metadata",
            map(vec![
                ("actor", string("agent-7")),
                ("validAt", string("2021-06-01")),
                ("request_id", Value::Int(42)),
                ("unrelated", string("ignored")),
            ]),
        )]);
        let got = tx_settings(&extra);
        assert!(got.contains(&("og.actor".into(), "agent-7".into())));
        assert!(got.contains(&("og.valid_at".into(), "2021-06-01".into())));
        assert!(got.contains(&("og.request_id".into(), "42".into())));
        assert_eq!(got.len(), 3);
    }

    #[test]
    fn no_metadata_no_settings() {
        assert!(tx_settings(&Value::Null).is_empty());
        assert!(tx_settings(&map(vec![("db", string("g"))])).is_empty());
    }
}

#[cfg(test)]
mod watch_tests {
    use super::*;

    /// `msg` as one Bolt chunk plus the end marker.
    fn chunked(msg: &[u8]) -> Vec<u8> {
        let mut out = (msg.len() as u16).to_be_bytes().to_vec();
        out.extend_from_slice(msg);
        out.extend_from_slice(&[0, 0]);
        out
    }

    #[test]
    fn reset_is_found_behind_a_pipelined_pull() {
        // PULL {n: -1}, then RESET.
        let mut buf = chunked(&[0xB1, PULL, 0xA1, 0x81, b'n', 0xFF]);
        assert!(!holds_reset(&buf));
        buf.extend(chunked(&[0xB0, RESET]));
        assert!(holds_reset(&buf));
    }

    #[test]
    fn a_reset_byte_inside_another_message_is_not_a_reset() {
        // A RUN whose query text happens to contain 0xB0 0x0F.
        let buf = chunked(&[0xB3, RUN, 0x82, 0xB0, RESET, 0xA0, 0xA0]);
        assert!(!holds_reset(&buf));
        // An incomplete chunk is not a message yet.
        assert!(!holds_reset(&[0x00, 0x02, 0xB0]));
    }

    #[test]
    fn driver_timeout_is_milliseconds_and_zero_means_none() {
        assert_eq!(tx_timeout(&map(vec![("tx_timeout", Value::Int(1500))])), Some(1500));
        assert_eq!(tx_timeout(&map(vec![("tx_timeout", Value::Int(0))])), Some(0));
        assert_eq!(tx_timeout(&map(vec![("tx_timeout", Value::Int(-1))])), None);
        assert_eq!(tx_timeout(&map(vec![("db", string("g"))])), None);
    }
}

#[cfg(test)]
mod plan_prefix_tests {
    use super::*;

    #[test]
    fn plan_prefixes_are_stripped() {
        assert_eq!(split_plan_prefix("EXPLAIN MATCH (n) RETURN n"), ("MATCH (n) RETURN n", true));
        assert_eq!(split_plan_prefix("  profile\tRETURN 1"), ("RETURN 1", true));
        assert_eq!(split_plan_prefix("EXPLAIN FABRIC MATCH (n) RETURN n"), ("EXPLAIN FABRIC MATCH (n) RETURN n", false));
        assert_eq!(split_plan_prefix("explainer"), ("explainer", false));
        assert_eq!(split_plan_prefix("RETURN 1"), ("RETURN 1", false));
    }

    #[test]
    fn multibyte_characters_in_the_first_bytes_do_not_panic() {
        // A character that straddles byte 7 (`explain`/`profile`) or byte 6 (`fabric`).
        for q in [
            "// 소스 확인\nMATCH (n) RETURN n",
            "WITH {수량: 3} AS m RETURN m.수량",
            "WITH '🙂' AS f RETURN f",
            "//🙂🙂\nRETURN 1",
            "가나다",
            "é",
        ] {
            assert_eq!(split_plan_prefix(q), (q, false), "{q}");
        }
        assert_eq!(split_plan_prefix("EXPLAIN //소스\nMATCH (n) RETURN n"), ("//소스\nMATCH (n) RETURN n", true));
        assert_eq!(split_plan_prefix("PROFILE 🙂🙂"), ("🙂🙂", true));
        assert_eq!(split_plan_prefix("EXPLAIN fabri🙂 x"), ("fabri🙂 x", true));
    }
}
