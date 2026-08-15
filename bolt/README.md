# Bolt gateway — spec 011

A Neo4j driver connects here and the application above it does not change.

```python
# was Neo4j
driver = GraphDatabase.driver("bolt://neo4j-host:7687", auth=basic_auth(user, pw))
# now Ontological — same driver, same sessions, same Cypher, same result objects
driver = GraphDatabase.driver("bolt://og-host:7687", auth=basic_auth(pg_role, pg_password))
```

The gateway holds no state: no parser, no planner, no cache, no user store.
Every query it receives goes to `og_cypher()`, so Cypher semantics, compilation,
error messages and transactions are spec 003's — reached through a different
transport, not a second implementation.

## Running it

```bash
cargo build --release

OG_BOLT_PGPORT=28816 OG_BOLT_PGDATABASE=og \
  ./target/release/ontological-bolt
```

| Variable | Default | Meaning |
|---|---|---|
| `OG_BOLT_LISTEN` | `0.0.0.0:7687` | where to accept Bolt connections |
| `OG_BOLT_PGHOST` | `localhost` | PostgreSQL host |
| `OG_BOLT_PGPORT` | `5432` | PostgreSQL port |
| `OG_BOLT_PGDATABASE` | `og` | PostgreSQL database |
| `OG_BOLT_GRAPH` | `default` | graph used when a session names no database |
| `OG_BOLT_ADVERTISED` | same as `OG_BOLT_LISTEN` | address returned in the routing table |

`start.sh` starts it alongside PostgreSQL. Nothing on the PostgreSQL path
depends on it: stop the gateway and every `og_cypher()` caller is unaffected.

## How the two worlds line up

| Neo4j | Here |
|---|---|
| database (`session(database="x")`) | **graph** `x` |
| user / password in `HELLO` | **PostgreSQL role** and its password — there is no second user store |
| `Node` | `{_id, _type, …}` from `og_cypher()`, re-encoded as a Bolt `Node` |
| `Relationship` | `{_id, _type, _src, _dst, …}`, re-encoded as a Bolt `Relationship` |
| explicit transaction | PostgreSQL transaction on the session's connection |
| `Neo4jError` | the compiler's message, verbatim, under a `Neo.ClientError.*` code |

Permissions, RLS and audit stay PostgreSQL's. A role that cannot see a row over
psql cannot see it over Bolt — the gateway never connects as anyone but the
authenticated user.

## Support matrix

Stated precisely, because "supported" without a matrix is not a claim
(constitution, technology constraints).

| | State |
|---|---|
| **Bolt 4.4** | supported — the version every current driver can negotiate |
| Bolt 3.x, 5.x | **not supported**; negotiation fails cleanly rather than half-working |
| Messages | `HELLO` `RUN` `PULL` `DISCARD` `BEGIN` `COMMIT` `ROLLBACK` `RESET` `GOODBYE` `ROUTE` |
| PackStream | null, bool, int (all widths), float, string, list, dictionary, structure |
| Graph types | `Node`, `Relationship` |
| `Path` | not encoded as a Path structure; a path variable arrives as a list of hops |
| Temporal / spatial types | **not supported** — a parameter carrying one is rejected, not silently mangled |
| Routing (`neo4j://`) | answers with this single server; a real routing table belongs to spec 007 |
| TLS | not terminated here; put a TLS proxy in front |
| `CALL {}`, APOC, GDS | not supported — spec 003 does not support them, and the transport does not add semantics |

Cypher coverage is spec 003's, unchanged: `docs/cypher.md` is the authority, and
`WITH`, `UNION` and `shortestPath` fail here exactly as they fail over psql.

## Tests

```bash
cargo test                      # PackStream round trips, chunk boundaries
python3 ../tests/neo4j-movies/run.py
```

The second one is the real gate: the official Neo4j Movie Graph sample, run
over Bolt with **Neo4j's own driver**, compared query by query against both the
PostgreSQL path and a live Neo4j. Testing our server with our own client would
not be evidence.
