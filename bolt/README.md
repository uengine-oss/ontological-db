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
| `OG_BOLT_STATEMENT_TIMEOUT` | unset: the engine's `og.statement_timeout` (2 min) | every session's statement limit: `300s`, `2min`, `120000` (ms), `0` for none |
| `OG_BOLT_ENGINE_SCHEMA` | unset: engine functions are called unqualified (resolved by `search_path`, which is `public` for the `ontological` extension) | schema to call the engine functions in, e.g. `oga_catalog` for `ontological_age`. Set it explicitly (`public`) when both editions are installed in one database |

## In front of Apache AGE (`OG_BOLT_BACKEND=age`)

The same gateway can stand in front of an unmodified Apache AGE 1.8.0
(PostgreSQL 15, e.g. the Supabase image in `age-supabase/`) instead of the
engine. The default (`ontological`) is unchanged: the five engine calls and
their SQL are the ones above.

| Variable | Default | Meaning |
|---|---|---|
| `OG_BOLT_BACKEND` | `ontological` | `age` to run Cypher through `ag_catalog.cypher()` |
| `OG_BOLT_AGE_CREATE_GRAPHS` | on | make a graph the first time a session names it (as the role that logged in, which then owns it — AGE needs that to add labels). `0`: answer `DatabaseNotFound` |
| `OG_BOLT_AGE_LOAD` | off | `LOAD 'age'` at login, for a server that does not preload AGE (superuser only) |
| `OG_BOLT_AGE_TRACE` | off | log every SQL statement sent to AGE and its time to stderr |

The session sets `search_path = ag_catalog, "$user", public` (AGE's generated
SQL needs it). `HELLO` answers `Neo4j/4.4.0 (age-bolt/<version>)`, without
"ontolog", so clients that send Ontological's fabric statements to an
Ontological gateway (Studio, robo-data-catalog, robo-data-analyzer) do not.

What happens to a statement (`src/age/`):

- it is parsed with the engine's own parser (`../cypher-syntax`, the engine's
  lexer, parser and AST built without pgrx), which gives the RETURN columns
  and read/write exactly as `og_cypher_columns()` / `og_cypher_check()` do;
- it is printed back as AGE's Cypher inside
  `SELECT * FROM ag_catalog.cypher('<graph>', $og$…$og$, $1) AS (c0 agtype, …)`,
  the parameters going as one agtype map in `$1` (binary: `0x01` + JSON);
- results are read as agtype text: `::vertex` → `Node`, `::edge` →
  `Relationship`, `::path` → `Path`, `1` and `1.0` kept apart. A node's id
  is AGE's graphid, and the element id a Bolt 4.4 driver derives from it is
  `toString(id(n))`, which `elementId(n)` is rewritten to.

Rewrites (each found by running Robo Architect's queries against AGE; see
`cypher.rs`, `split.rs` and `reach.rs` for the measurement behind each):

| Neo4j | sent to AGE |
|---|---|
| `datetime()`, `date()`; `datetime($p)`; `datetime() - duration({minutes: $m})` | the statement's clock, `$p` written as PostgreSQL writes a timestamptz, the shifted instant — as ISO 8601 text in parameters |
| `randomUUID()` | `toString(pg_catalog.concat(pg_catalog.gen_random_uuid()))` |
| `n:L` in an expression; `(n:A\|B)`, `-[:A\|B]-` | `label(n) = 'L'`; the choice moved into the clause's WHERE |
| `any/all/none/single(x IN l WHERE p)` | `size([x IN l WHERE p]) > 0` … (AGE's own crash the backend when `p` reads a matched node) |
| `n {.a, .b}` | `CASE WHEN n IS NULL THEN null ELSE {a: n.a, b: n.b} END` (Neo4j's answer for null and for an absent key) |
| `coalesce(a, b)` over nodes | `coalesce(head([a]), head([b]))` |
| `SET n += $map` | `WITH …, $map AS p SET n += p` |
| an aggregation next to a list comprehension | three projections: lists, aggregates, the rest |
| variables named `id`, `properties`, `start_id`, `end_id` | renamed (they clash with the label tables' columns) |
| a leading `OPTIONAL MATCH` | `WITH 1 AS …` in front (AGE returns no row when nothing matches) |
| `ORDER BY m.k` of a projected map `m` | the key read from where the map got it |
| `RETURN *` | the variables in scope, sorted |
| `FOREACH`, `CALL { … }`, `SET n:L` / `REMOVE n:L`, a SET after a MERGE that has ON CREATE/ON MATCH items | cut into several statements in one transaction, the rows carried between them as a parameter (vertices and edges by id) — AGE has no FOREACH or CALL {}, keeps one label per vertex, and drops a SET that follows such a MERGE |
| `CREATE CONSTRAINT … IS UNIQUE`, `CREATE INDEX` | unique / plain expression indexes on the label's table (made if missing); `IS NOT NULL`, full-text and vector are accepted and not enforced |
| `CALL db.labels()`, `db.relationshipTypes()`, `db.propertyKeys()`; `SHOW GRAPHS`, `CREATE GRAPH g` | answered from `ag_catalog` |
| a variable-length relationship without exactly one type over 3 hops (`-[:A\|B\|…*1..30]-`, `<-[*1..30]-`) where only the nodes it reaches matter: in `EXISTS { }` between two bound nodes, or as the only pattern of a MATCH followed by `DISTINCT`, `DISTINCT` aggregates (and `min`/`max`) or `LIMIT 1` | walked by the gateway (`reach.rs`): the start nodes' ids from the clauses before it, a breadth-first walk with one SQL statement per hop on the label tables' `start_id`/`end_id`, then the statement with the pattern replaced by `id(t) IN $reached[id(s)]`. Neo4j's trail semantics: the start node counts as reached only over a closed trail (a cycle through it) within the bound |

Refused by name: such a relationship where the trails themselves matter (a
relationship or path variable on it, `count(*)` or rows per trail after it, a
lower bound above 1, two of them in one statement — AGE enumerates every
trail; `*1..30` over ten types ran the server out of memory), a node with two
labels, and Ontological's fabric statements.

`summary.counters` come from PostgreSQL's per-transaction statistics of the
graph's label tables (`pg_stat_xact_*`), read before and after the statement:
nodes and relationships created and deleted are exact; `properties-set`
counts updated elements, not properties. A write sent without `BEGIN` runs in
a transaction of its own for that reason.

## Time limits and cancellation

A statement is cancelled — in PostgreSQL and, on virtual nodes, at the source —
when any of these happens:

| Event | Bolt answer |
|---|---|
| it runs past its limit: the driver's transaction timeout (`tx_timeout`, ms) if it sent one, otherwise the session's (`OG_BOLT_STATEMENT_TIMEOUT`, otherwise the engine's two minutes) | `FAILURE Neo.ClientError.Transaction.TransactionTimedOut`, with what to change; the session goes on after `RESET`, as after any failure |
| the client closes the connection, or it breaks | nobody to answer — the query stops instead of running on |
| the client sends `RESET` while it runs | `FAILURE Neo.TransientError.Transaction.Terminated`, then `IGNORED`, `SUCCESS` |
| the backend is cancelled from outside (`pg_cancel_backend`) | `FAILURE Neo.TransientError.Transaction.Terminated` |
| the gateway itself dies | PostgreSQL notices within about two seconds (`client_connection_check_interval`, not available on Windows) |

The limit is the engine's (`og.statement_timeout`, see `docs/cypher.md`): the
gateway only passes the driver's value on. While a query runs a second thread
reads the client socket — that is the only way to see a close behind the PULL a
driver has already pipelined — and sends PostgreSQL a cancel request when the
client is gone or sends `RESET`. It also cancels a few seconds after the limit
if the engine has not, as a backstop.

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
| Bolt 3.x, 5.x | **not spoken**. This is not the limit it sounds like: a driver proposes a *range*, so the current Python driver (6.2) settles on 4.4 and connects. Negotiation fails cleanly for anything that cannot |
| Messages | `HELLO` `RUN` `PULL` `DISCARD` `BEGIN` `COMMIT` `ROLLBACK` `RESET` `GOODBYE` `ROUTE` |
| PackStream | null, bool, int (all widths), float, string, list, dictionary, structure |
| Graph types | `Node`, `Relationship` |
| `Path` | not encoded as a Path structure; a path variable arrives as a list of hops |
| Temporal / spatial types | **not supported** — a parameter carrying one is rejected, not silently mangled |
| Routing (`neo4j://`) | answers with this single server; a real routing table belongs to spec 007 |
| TLS | not terminated here; put a TLS proxy in front |
| `EXPLAIN` / `PROFILE` | accepted: the query is classified, not run, and the summary's `type` is `r` or `w`. No plan — see `docs/cypher.md` |
| `summary.counters` | nodes/relationships created and deleted, properties set, labels added, indexes and constraints added and removed |
| `CALL {}`, GDS | not supported — spec 003 does not support them, and the transport does not add semantics |
| `CALL` procedures, APOC | whatever spec 003 supports; `docs/cypher.md` has the registry |

Cypher coverage is spec 003's, unchanged: `docs/cypher.md` is the authority, and
`UNION` and `shortestPath` fail here exactly as they fail over psql.

`EXPLAIN` is the one place the gateway looks at a query at all, and it still
does not parse one: it asks `og_cypher_check()` whether the statement writes, so
the read/write verdict a client acts on is the engine's, reached through a
second transport rather than reimplemented behind it.

## Driving it from an MCP server

Neo4j's published MCP server, `mcp-neo4j-cypher`, runs against this gateway
unmodified — it is only a Neo4j driver plus three tools. What that takes, and
where it still falls short, is worked end to end in
[`examples/meeting-rooms/`](../examples/meeting-rooms/), whose `verify_mcp.py`
prints the compatibility matrix as a test.

## Tests

```bash
cargo test                      # PackStream round trips, chunk boundaries, the AGE translation
python3 ../tests/common/neo4j-movies/run.py
```

The second one is the real gate: the official Neo4j Movie Graph sample, run
over Bolt with **Neo4j's own driver**, compared query by query against both the
PostgreSQL path and a live Neo4j. Testing our server with our own client would
not be evidence.
