//! Bolt protocol gateway for Ontological — spec 011.
//!
//! Speaks Bolt to Neo4j drivers and executes what they send with `og_cypher()`.
//! It holds no state of its own: no parser, no planner, no cache, no user
//! store. Cypher is never interpreted here — one query path, spec 003's.
//!
//!     ontological-bolt
//!
//! Configuration is environment only:
//!
//!     OG_BOLT_LISTEN      default 127.0.0.1:7687
//!     OG_BOLT_PGHOST      default localhost
//!     OG_BOLT_PGPORT      default 5432
//!     OG_BOLT_PGDATABASE  default og
//!     OG_BOLT_GRAPH       default "default" — used when a session names no database
//!     OG_BOLT_MAX_SESSIONS default 256
//!
//! The listen default is loopback. Bolt carries the password in the clear on
//! the way in — there is no TLS here — and this process authenticates by
//! opening a PostgreSQL connection as whoever the client claimed to be, so a
//! reachable port is a credential-sniffing opportunity and a login oracle.
//! Binding somewhere else is a decision, and `OG_BOLT_LISTEN` is how you make
//! it; `start.sh` makes it explicitly, because there the container's network
//! namespace is the boundary rather than the bind address.

mod packstream;
mod session;

use std::env;
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

/// How many sessions may be open at once.
///
/// A thread per connection and a PostgreSQL connection per thread means an
/// accept flood costs a stack and a backend each, and nothing here said no.
/// Refusing at the door is cheaper than discovering the limit as
/// `max_connections` on the database.
const MAX_SESSIONS: usize = 256;

/// Decrements the live-session count however the thread ends — returning,
/// erroring, or panicking.
struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct Config {
    pub pg_host: String,
    pub pg_port: u16,
    pub pg_database: String,
    pub default_graph: String,
    pub advertised: String,
}

fn env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn main() {
    let listen = env_or("OG_BOLT_LISTEN", "127.0.0.1:7687");
    let max_sessions = env_or("OG_BOLT_MAX_SESSIONS", "")
        .parse()
        .unwrap_or(MAX_SESSIONS);
    let config = Config {
        pg_host: env_or("OG_BOLT_PGHOST", "localhost"),
        pg_port: env_or("OG_BOLT_PGPORT", "5432").parse().unwrap_or(5432),
        pg_database: env_or("OG_BOLT_PGDATABASE", "og"),
        default_graph: env_or("OG_BOLT_GRAPH", "default"),
        advertised: env_or("OG_BOLT_ADVERTISED", &listen),
    };

    let listener = match TcpListener::bind(&listen) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("ontological-bolt: cannot bind {listen}: {e}");
            std::process::exit(1);
        }
    };
    eprintln!(
        "ontological-bolt: listening on {listen}, \
         forwarding to postgres://{}:{}/{} (default graph '{}')",
        config.pg_host, config.pg_port, config.pg_database, config.default_graph
    );

    let config = Arc::new(config);
    let live = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("ontological-bolt: accept failed: {e}");
                continue;
            }
        };
        if live.load(Ordering::SeqCst) >= max_sessions {
            // Dropping the stream closes it. Saying more would mean decoding
            // what the peer sent, which is the work being refused.
            eprintln!("ontological-bolt: at {max_sessions} sessions, refusing a connection");
            drop(stream);
            continue;
        }
        live.fetch_add(1, Ordering::SeqCst);
        let slot = Slot(live.clone());
        let config = config.clone();
        // A connection per thread, a PostgreSQL connection per session: the
        // concurrency limit is PostgreSQL's, which is the one that matters.
        thread::spawn(move || {
            let _slot = slot;
            let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_default();
            if let Err(e) = session::serve(stream, &config) {
                // A driver closing its connection is the normal ending, not an error.
                if e.kind() != std::io::ErrorKind::UnexpectedEof {
                    eprintln!("ontological-bolt: session {peer} ended: {e}");
                }
            }
        });
    }
}
