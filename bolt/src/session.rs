//! One Bolt connection: handshake, message loop, and the mapping between
//! `og_cypher()` results and Bolt values (spec 011, FR-005…FR-017).

use crate::packstream::{self as ps, map, string, Value};
use crate::Config;
use postgres::{Client, NoTls};
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::TcpStream;

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
    };
    sess.run(&mut stream)
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
            let msg = ps::read_message(stream)?;
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
            RUN => self.run_query(fields),
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
        let client = cfg.connect(NoTls).map_err(|e| {
            Failure::client("Neo.ClientError.Security.Unauthorized", pg_message(&e))
        })?;
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
        self.state = State::Ready;
        Ok(Value::Map(BTreeMap::new()))
    }

    fn begin(&mut self, extra: Value) -> Result<Value, Failure> {
        self.select_graph(&extra);
        self.sql("BEGIN")?;
        self.in_tx = true;
        Ok(Value::Map(BTreeMap::new()))
    }

    fn end_tx(&mut self, verb: &str) -> Result<Value, Failure> {
        self.sql(verb)?;
        self.in_tx = false;
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

    fn run_query(&mut self, mut fields: Vec<Value>) -> Result<Value, Failure> {
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

        // Field order comes from the parser, not from the row: jsonb sorts its
        // keys, so the row cannot tell us what the query asked for (FR-010).
        let columns: Vec<String> = {
            let pg = self.client()?;
            let row = pg
                .query_one("SELECT og_cypher_columns($1::text)", &[&query])
                .map_err(|e| Failure::from_pg(&e))?;
            row.get(0)
        };

        let rows = {
            let graph = self.graph.clone();
            let pg = self.client()?;
            pg.query(
                "SELECT og_cypher($1::text, $2::text, $3::text::jsonb)::text",
                &[&graph, &query, &params_json],
            )
            .map_err(|e| Failure::from_pg(&e))?
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
            meta.push(("type", string("r")));
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

// ----------------------------------------------------------------- failures

struct Failure {
    code: String,
    message: String,
}

impl Failure {
    fn client(code: &str, message: String) -> Self {
        Failure { code: code.to_string(), message }
    }

    /// The compiler's message is the valuable part — it names the construct and
    /// suggests the alternative (spec 003 FR-008), and an agent retries on it
    /// (principle VIII). It is passed through verbatim; only the code is ours.
    fn from_pg(e: &postgres::Error) -> Self {
        let message = pg_message(e);
        let code = if message.contains("not supported")
            || message.contains("expected")
            || message.contains("unknown label")
            || message.contains("is not defined")
        {
            "Neo.ClientError.Statement.SyntaxError"
        } else if message.contains("does not exist") {
            "Neo.ClientError.Database.DatabaseNotFound"
        } else if message.contains("permission denied") {
            "Neo.ClientError.Security.Forbidden"
        } else {
            "Neo.ClientError.Statement.ArgumentError"
        };
        Failure::client(code, message)
    }

    fn into_message(self) -> Value {
        Value::Struct(
            FAILURE,
            vec![map(vec![("code", string(self.code)), ("message", string(self.message))])],
        )
    }
}

fn pg_message(e: &postgres::Error) -> String {
    e.as_db_error().map(|d| d.message().to_string()).unwrap_or_else(|| e.to_string())
}
