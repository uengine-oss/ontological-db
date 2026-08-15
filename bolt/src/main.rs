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
//!     OG_BOLT_LISTEN      default 0.0.0.0:7687
//!     OG_BOLT_PGHOST      default localhost
//!     OG_BOLT_PGPORT      default 5432
//!     OG_BOLT_PGDATABASE  default og
//!     OG_BOLT_GRAPH       default "default" — used when a session names no database

mod packstream;
mod session;

use std::env;
use std::net::TcpListener;
use std::thread;

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
    let listen = env_or("OG_BOLT_LISTEN", "0.0.0.0:7687");
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

    let config = std::sync::Arc::new(config);
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("ontological-bolt: accept failed: {e}");
                continue;
            }
        };
        let config = config.clone();
        // A connection per thread, a PostgreSQL connection per session: the
        // concurrency limit is PostgreSQL's, which is the one that matters.
        thread::spawn(move || {
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
