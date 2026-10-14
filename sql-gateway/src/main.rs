//! PostgreSQL-protocol gateway for Ontological.
//!
//! A PostgreSQL client — psql, a JDBC tool, a driver — connects here instead of
//! to the server, and can then type what the server's own grammar has no room
//! for (ADR-008):
//!
//! ```text
//! CREATE DATASOURCE erp WITH ENGINE = 'postgres', PARAMETERS = {...};
//! SHOW TABLES FROM erp;
//! CREATE VIRTUAL NODE Supplier FROM erp.supplier KEY supplier_id;
//! MATCH (s:Supplier) WHERE s.country = 'China' RETURN s.name;
//! ```
//!
//! It is a relay, not a server. Every byte of the protocol passes through
//! untouched — authentication, transactions, prepared statements, COPY — except
//! the SQL text of a statement that opens like a fabric statement or a Cypher
//! query. For those it asks the server for the translation
//! (`og_fabric_translate`) and sends that in the statement's place. The grammar
//! lives in the extension; this process knows only which statements to ask
//! about. Anything it does not recognise is, by construction, ordinary
//! PostgreSQL.
//!
//! Environment:
//!
//!     OG_SQL_LISTEN   default 127.0.0.1:5433
//!     OG_SQL_PGHOST   default localhost
//!     OG_SQL_PGPORT   default 5432
//!
//! The server sees every client as coming from this process's address. Where
//! `pg_hba.conf` trusts that address, the gateway must not be reachable by
//! anyone the server would not trust — hence the loopback default.

use std::env;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

const SSL_REQUEST: u32 = 80877103;
const GSSENC_REQUEST: u32 = 80877104;
const CANCEL_REQUEST: u32 = 80877102;

fn env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn main() {
    let listen = env_or("OG_SQL_LISTEN", "127.0.0.1:5433");
    let backend = format!("{}:{}", env_or("OG_SQL_PGHOST", "localhost"), env_or("OG_SQL_PGPORT", "5432"));
    let listener = TcpListener::bind(&listen).unwrap_or_else(|e| {
        eprintln!("ontological-sql: cannot bind {listen}: {e}");
        std::process::exit(1);
    });
    eprintln!("ontological-sql: listening on {listen}, relaying to postgres://{backend}");
    for stream in listener.incoming() {
        let Ok(client) = stream else { continue };
        let backend = backend.clone();
        thread::spawn(move || {
            if let Err(e) = session(client, &backend) {
                if e.kind() != io::ErrorKind::UnexpectedEof {
                    eprintln!("ontological-sql: session ended: {e}");
                }
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Which statements to ask about
// ---------------------------------------------------------------------------

/// Leading whitespace and comments off.
fn lead(sql: &str) -> &str {
    let mut s = sql.trim_start();
    loop {
        if let Some(rest) = s.strip_prefix("--") {
            s = rest.split_once('\n').map_or("", |(_, r)| r).trim_start();
        } else if let Some(rest) = s.strip_prefix("/*") {
            s = rest.split_once("*/").map_or("", |(_, r)| r).trim_start();
        } else {
            return s;
        }
    }
}

/// True for a statement PostgreSQL would reject or misread and the fabric
/// grammar claims. Erring towards false is safe: the statement then reaches
/// the server as it was typed.
fn is_fabric(sql: &str) -> bool {
    let s = lead(sql);
    let mut words = s
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .map(|w| w.to_ascii_uppercase());
    let (w0, w1) = (words.next().unwrap_or_default(), words.next().unwrap_or_default());
    let after_first = s.get(w0.len()..).unwrap_or("").trim_start();
    match w0.as_str() {
        "MATCH" | "OPTIONAL" | "UNWIND" | "RETURN" | "DETACH" | "MATERIALIZE" | "USE" => true,
        // `MERGE INTO …` is SQL; `MERGE (n:…)` is Cypher. Likewise CREATE.
        "MERGE" => after_first.starts_with('('),
        "CREATE" => {
            after_first.starts_with('(')
                || matches!(w1.as_str(), "DATASOURCE" | "VIRTUAL" | "GRAPH")
                || (w1 == "DATABASE" && s.to_ascii_uppercase().split_whitespace().any(|w| w.starts_with("ENGINE")))
        }
        "DROP" => matches!(w1.as_str(), "DATASOURCE" | "VIRTUAL"),
        "SHOW" => matches!(w1.as_str(), "DATASOURCES" | "TABLES" | "VIRTUAL" | "GRAPHS"),
        "REFRESH" => w1 == "DATASOURCE",
        "EXPLAIN" => w1 == "FABRIC",
        _ => false,
    }
}

/// Dollar-quote a text that may itself contain dollar quotes.
fn dq(s: &str) -> String {
    let mut n = 0;
    loop {
        let tag = if n == 0 { "$ogq$".to_string() } else { format!("$ogq{n}$") };
        if !s.contains(&tag) {
            return format!("{tag}{s}{tag}");
        }
        n += 1;
    }
}

// ---------------------------------------------------------------------------
// Wire framing
// ---------------------------------------------------------------------------

struct Msg {
    tag: u8,
    body: Vec<u8>,
}

fn read_msg(r: &mut impl Read) -> io::Result<Msg> {
    let mut head = [0u8; 5];
    r.read_exact(&mut head)?;
    let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if len < 4 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad message length"));
    }
    let mut body = vec![0u8; len - 4];
    r.read_exact(&mut body)?;
    Ok(Msg { tag: head[0], body })
}

fn write_msg(w: &mut impl Write, tag: u8, body: &[u8]) -> io::Result<()> {
    let mut out = Vec::with_capacity(body.len() + 5);
    out.push(tag);
    out.extend_from_slice(&((body.len() + 4) as u32).to_be_bytes());
    out.extend_from_slice(body);
    w.write_all(&out)
}

fn cstr(bytes: &[u8]) -> (&str, &[u8]) {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    (std::str::from_utf8(&bytes[..end]).unwrap_or(""), bytes.get(end + 1..).unwrap_or(&[]))
}

fn query_body(sql: &str) -> Vec<u8> {
    let mut b = sql.as_bytes().to_vec();
    b.push(0);
    b
}

// ---------------------------------------------------------------------------
// One session
// ---------------------------------------------------------------------------

#[derive(Default)]
struct State {
    /// Requests sent whose `ReadyForQuery` has not come back. The server is
    /// idle, and a question of our own may be put to it, only at zero.
    awaiting: usize,
    /// While set, the server's messages answer the gateway, not the client.
    asking: bool,
    answer: Option<Vec<Msg>>,
    captured: Vec<Msg>,
    closed: bool,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

fn session(mut client: TcpStream, backend_addr: &str) -> io::Result<()> {
    // Startup: untyped packets until the real StartupMessage.
    let mut backend = loop {
        let mut len = [0u8; 4];
        client.read_exact(&mut len)?;
        let n = u32::from_be_bytes(len) as usize;
        if !(8..=10_000).contains(&n) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "bad startup packet"));
        }
        let mut rest = vec![0u8; n - 4];
        client.read_exact(&mut rest)?;
        let code = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]);
        match code {
            // No TLS here; the client carries on in the clear or gives up.
            SSL_REQUEST | GSSENC_REQUEST => client.write_all(b"N")?,
            CANCEL_REQUEST => {
                let mut b = TcpStream::connect(backend_addr)?;
                b.write_all(&len)?;
                b.write_all(&rest)?;
                return Ok(());
            }
            _ => {
                let mut b = TcpStream::connect(backend_addr)?;
                b.write_all(&len)?;
                b.write_all(&rest)?;
                break b;
            }
        }
    };
    client.set_nodelay(true).ok();
    backend.set_nodelay(true).ok();

    // The startup itself ends in a ReadyForQuery.
    let shared = Arc::new(Shared { state: Mutex::new(State { awaiting: 1, ..State::default() }), changed: Condvar::new() });
    let to_client = Arc::new(Mutex::new(client.try_clone()?));

    // Server → client.
    let down = {
        let (shared, to_client) = (shared.clone(), to_client.clone());
        let mut from_backend = backend.try_clone()?;
        thread::spawn(move || {
            let result = (|| -> io::Result<()> {
                loop {
                    let m = read_msg(&mut from_backend)?;
                    let mut st = shared.state.lock().unwrap();
                    // Notices, parameter changes and notifications are the
                    // client's whoever asked the question.
                    let asynchronous = matches!(m.tag, b'N' | b'S' | b'A');
                    if st.asking && !asynchronous {
                        let done = m.tag == b'Z';
                        st.captured.push(m);
                        if done {
                            st.asking = false;
                            st.answer = Some(std::mem::take(&mut st.captured));
                            shared.changed.notify_all();
                        }
                        continue;
                    }
                    if m.tag == b'Z' {
                        st.awaiting = st.awaiting.saturating_sub(1);
                        shared.changed.notify_all();
                    }
                    // Not written under the state lock: a client slow to read
                    // must not stall the other direction. The writer's own
                    // lock keeps whole messages from interleaving.
                    drop(st);
                    write_msg(&mut *to_client.lock().unwrap(), m.tag, &m.body)?;
                }
            })();
            let mut st = shared.state.lock().unwrap();
            st.closed = true;
            shared.changed.notify_all();
            to_client.lock().unwrap().shutdown(Shutdown::Both).ok();
            result
        })
    };

    // Client → server.
    let result = up(&mut client, &mut backend, &shared, &to_client);
    backend.shutdown(Shutdown::Both).ok();
    client.shutdown(Shutdown::Both).ok();
    down.join().ok();
    result
}

enum Translation {
    Sql(String),
    /// The server's ErrorResponse, and the transaction status that followed it.
    Refused(Vec<u8>, u8),
}

/// Put the statement to the server and wait for what it translates to. Only
/// called with the server idle.
fn ask(backend: &mut TcpStream, shared: &Shared, statement: &str) -> io::Result<Translation> {
    {
        let mut st = shared.state.lock().unwrap();
        while st.awaiting > 0 && !st.closed {
            st = shared.changed.wait(st).unwrap();
        }
        if st.closed {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        st.asking = true;
        st.answer = None;
    }
    write_msg(backend, b'Q', &query_body(&format!("SELECT og_fabric_translate({})", dq(statement))))?;
    let answer = {
        let mut st = shared.state.lock().unwrap();
        while st.answer.is_none() && !st.closed {
            st = shared.changed.wait(st).unwrap();
        }
        st.answer.take().ok_or(io::ErrorKind::UnexpectedEof)?
    };
    let status = answer.iter().rev().find(|m| m.tag == b'Z').and_then(|m| m.body.first().copied()).unwrap_or(b'I');
    if let Some(e) = answer.iter().find(|m| m.tag == b'E') {
        return Ok(Translation::Refused(e.body.clone(), status));
    }
    // DataRow: int16 column count, then int32 length + bytes per column.
    let row = answer.iter().find(|m| m.tag == b'D').ok_or(io::ErrorKind::InvalidData)?;
    let len = i32::from_be_bytes([row.body[2], row.body[3], row.body[4], row.body[5]]).max(0) as usize;
    let sql = String::from_utf8_lossy(&row.body[6..6 + len]).into_owned();
    Ok(Translation::Sql(sql))
}

fn up(client: &mut TcpStream, backend: &mut TcpStream, shared: &Shared, to_client: &Mutex<TcpStream>) -> io::Result<()> {
    // Extended-protocol messages sent since the last Sync. In the middle of
    // such a batch the server cannot be asked anything without disturbing it.
    let mut unsynced = false;
    let sent = |n: usize| shared.state.lock().unwrap().awaiting += n;
    // Where asking is impossible, the statement still runs — through the
    // function that translates and executes in one step, as jsonb rows.
    let fallback = |sql: &str| format!("SELECT * FROM og_fabric({})", dq(sql));

    loop {
        let m = read_msg(client)?;
        match m.tag {
            b'Q' => {
                let (sql, _) = cstr(&m.body);
                if is_fabric(sql) {
                    if unsynced {
                        sent(1);
                        write_msg(backend, b'Q', &query_body(&fallback(sql)))?;
                        continue;
                    }
                    match ask(backend, shared, sql)? {
                        Translation::Sql(t) => {
                            sent(1);
                            write_msg(backend, b'Q', &query_body(&t))?;
                        }
                        Translation::Refused(err, status) => {
                            let mut c = to_client.lock().unwrap();
                            write_msg(&mut *c, b'E', &err)?;
                            write_msg(&mut *c, b'Z', &[status])?;
                        }
                    }
                } else {
                    sent(1);
                    write_msg(backend, b'Q', &m.body)?;
                }
            }
            b'P' => {
                let (name, rest) = cstr(&m.body);
                let (sql, params) = cstr(rest);
                if is_fabric(sql) {
                    let translated = if unsynced {
                        fallback(sql)
                    } else {
                        match ask(backend, shared, sql)? {
                            Translation::Sql(t) => t,
                            // A refusal has to reach the client in step with
                            // its own Bind/Execute, so let the server raise it
                            // again when the statement is run.
                            Translation::Refused(..) => format!("SELECT og_fabric_translate({})", dq(sql)),
                        }
                    };
                    let mut body = name.as_bytes().to_vec();
                    body.push(0);
                    body.extend_from_slice(translated.as_bytes());
                    body.push(0);
                    body.extend_from_slice(params);
                    write_msg(backend, b'P', &body)?;
                } else {
                    write_msg(backend, b'P', &m.body)?;
                }
                unsynced = true;
            }
            b'S' => {
                unsynced = false;
                sent(1);
                write_msg(backend, b'S', &m.body)?;
            }
            // A fast-path function call answers with its own ReadyForQuery.
            b'F' => {
                sent(1);
                write_msg(backend, b'F', &m.body)?;
            }
            b'X' => {
                write_msg(backend, b'X', &m.body)?;
                return Ok(());
            }
            b'B' | b'E' | b'D' | b'C' | b'H' => {
                unsynced = true;
                write_msg(backend, m.tag, &m.body)?;
            }
            // Passwords, SASL, COPY data: none of the gateway's business.
            _ => write_msg(backend, m.tag, &m.body)?,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_fabric;

    #[test]
    fn fabric_statements_are_recognised() {
        for s in [
            "CREATE DATASOURCE erp WITH ENGINE = 'postgres', PARAMETERS = {}",
            "create database erp with engine='mysql', parameters = {}",
            "  -- note\n SHOW DATASOURCES;",
            "SHOW TABLES FROM erp",
            "CREATE VIRTUAL NODE A FROM erp.a KEY id",
            "DROP VIRTUAL RELATIONSHIP R",
            "MATCH (n) RETURN n",
            "CREATE (:Region {name: 'x'})",
            "MERGE (n:Region {name: 'x'})",
            "EXPLAIN FABRIC MATCH (n) RETURN n",
            "USE GRAPH scm",
            "CREATE GRAPH scm",
            "SHOW GRAPHS",
            "REFRESH DATASOURCE erp",
        ] {
            assert!(is_fabric(s), "{s}");
        }
    }

    #[test]
    fn ordinary_sql_is_left_alone() {
        for s in [
            "SELECT 1",
            "CREATE TABLE t (id int)",
            "CREATE DATABASE app",
            "CREATE DATABASE app WITH OWNER = me",
            "DROP TABLE t",
            "SHOW search_path",
            "SHOW ALL",
            "MERGE INTO t USING s ON t.id = s.id WHEN MATCHED THEN DO NOTHING",
            "REFRESH MATERIALIZED VIEW mv",
            "EXPLAIN SELECT 1",
            "WITH x AS (SELECT 1) SELECT * FROM x",
            "",
        ] {
            assert!(!is_fabric(s), "{s}");
        }
    }
}
