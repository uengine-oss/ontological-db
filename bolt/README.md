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
| `OG_BOLT_AGE_CREATE_GRAPHS` | on | make a graph the first time a session names it (as the role that logged in, which then owns it — AGE needs that to add labels). `0`: answer `Neo.ClientError.Database.DatabaseNotFound` ("… make it with CREATE GRAPH x"); `SHOW GRAPHS` and `CREATE GRAPH x` work either way, whatever database the session names |
| `OG_BOLT_AGE_LOAD` | off | `LOAD 'age'` at login, for a server that does not preload AGE (superuser only) |
| `OG_BOLT_AGE_TRACE` | off | log every SQL statement sent to AGE and its time, and each statement's plan (cypher, steps, reach, ddl …), to stderr |
| `OG_BOLT_AGE_COUNTERS` | on | read `summary.counters` (two short SQL statements around every write); off: writes report no counters |

A Bolt database is the AGE graph of the same name, except a name of fewer
than 3 characters, which AGE refuses ("graph name is invalid", SUITES N10;
Neo4j refuses those too, but Robo's launcher makes 2-character projects,
`^[a-z][a-z0-9_]{1,39}$`): `ab` is kept in the AGE graph `_og_ab`. `SHOW
GRAPHS` lists it as `ab`, `CREATE GRAPH ab` makes `_og_ab`, and every
statement of a session on `ab` runs there (`mod.rs` `age_graph_name`). No
client name can land there by itself: Neo4j's database names start with a
letter, and `_og_` followed by one or two characters is refused as a database
name. Names of 3 characters or more are the graph of the same name, as
before. `CREATE GRAPH` of a graph that is there already answers
`Neo.ClientError.Database.ExistingDatabaseFound` (AGE gives it 3F000, the
code of a missing schema). An error message from AGE itself names the graph
as AGE has it (`_og_ab`).

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
| a DateTime parameter with a zone id (Bolt `0x66`; SUITES B9: the zone was dropped and the wall time read as UTC, nine hours off for Asia/Seoul) | its instant, asked of PostgreSQL's zone database before the statement (`timezone(zone, local::timestamp)`, one query per distinct value — `run.rs` `age_params`), as the canonical UTC text the other date-times take. A DateTime with an offset already says its instant and is left alone |
| `randomUUID()` | `toString(pg_catalog.concat(pg_catalog.gen_random_uuid()))` |
| `id(x) IN list` (AGE: `id(n) IN $param` kills the backend, signal 11; `id(n) IN [1, 2]` fails "not a common type" — ROBO 11.4) | `id(coalesce(x)) IN list` |
| `elementId(x) = e`, `e = elementId(x)` (STUDIO G4: `toString(id(…)) = e` reads every row, 28 ms → 2 ms at 25k vertices) | `id(x) = $__og_eid_p` / `id(x) = 12` when the parameter or literal is an integer's text; `id(x) = toInteger(e) AND toString(id(coalesce(x))) = e` for an expression (the index finds it, the text decides); the text comparison for anything else. `elementId(x) IN list` stays a text comparison (`id(x) IN` is the crash above) |
| `n:L` in an expression; `(n:A\|B)`, `-[:A\|B]-` | `label(n) = 'L'`, or `label(n) IN ['L', the labels whose tables inherit L's]` when there are any (*Label hierarchy* below); the choice moved into the clause's WHERE — for a node, `label(n) IN [A, B, the labels below them]` |
| `(n:A:B)` in MATCH / OPTIONAL MATCH / EXISTS — both labels (SUITES B1: was read as either, `MATCH (n:Lamp:Part)` counted Lamps and Parts; STUDIO G2: `(n:_Entity:Customer)` became a scan of every vertex, 6–10 ms at 25k, where one table takes 1 ms) | `(n:B)` — the last label's table, which AGE scans together with the tables inheriting it — with nothing more when every one of those tables has A too (`(n:_Entity:Customer)` once `Customer` inherits `_Entity`), `label(n) IN [those that do]` when some do, `false` when none does; `(n:A)` for `(n:A:A)` (`cypher.rs` `node_label_test`) |
| `CREATE` / `MERGE (n:A:B:C {…})` (STUDIO G1, ANALYZER §5.1: refused before — the analyzer's every node write, Studio's entity, ingest and publish writes) | `(n:C {…})`, and before it, in its transaction, the label tables that are missing (`create_vlabel`) and the inheritance C's table lacks (`ALTER TABLE "g"."C" INHERIT "g"."A"`) — *Label hierarchy* below (`hierarchy.rs`) |
| `labels(x)` (STUDIO G3: `["Customer"]` where Neo4j has `["Customer", "_Entity"]`; `$class IN labels(n)` found nothing) | `(CASE label(x) WHEN 'Customer' THEN ['Customer', '_Entity'] … ELSE labels(x) END)`, a branch for each label whose table inherits others, nearest first (`labels(n)[0]` is the specific label); as written when no table inherits |
| `any/all/none/single(x IN l WHERE p)` | `size([x IN l WHERE p]) > 0` … (AGE's own crash the backend when `p` reads a matched node) |
| `n {.a, .b}` | `CASE WHEN n IS NULL THEN null ELSE {a: n.a, b: n.b} END` (Neo4j's answer for null and for an absent key) |
| `n {.*, k: e, .p}` (ANALYZER §5.4: AGE builds `age_properties(n) + agtype_build_map_nonull(…)` — `n {.*, embedding: null}` kept the stored `embedding`, `n {.*, id: 'x'}` came back as `{id: 'x'}` alone) | `CASE WHEN n IS NULL THEN null ELSE (properties(n) - ['k', 'p']) + {k: e, p: n.p} END`: the entries' keys taken out of the properties first (`agtype - [keys]`), then the entries as a map literal, which keeps a null entry (`cypher.rs` `all_properties_and`). `n {.*}` alone is sent as it is |
| `coalesce(a, b)` over nodes | `coalesce(head([a]), head([b]))` |
| `SET n += $map`; `SET r = item.props`, `SET n += item.props` (ANALYZER §5.3: AGE takes a map literal, a function call or a variable there — "SET clause expects a map" for a parameter or a property access) | `WITH …, $map AS __og_p1 SET n += __og_p1`, `WITH …, item.props AS __og_setv1 SET r = __og_setv1` (`cypher.rs` `bind_map_params`); left as written when the value reads what a MERGE itself binds |
| a MERGE pattern property that is null (SUITES N7: AGE makes an element without the key, a new one every time; Neo4j refuses) | known now (a literal, a parameter given, a field of one): refused with Neo4j's `SemanticError` "Cannot merge the following node because of null property value for 'key': (:W {key: null})" ("…relationship…"). Computed (`r.missing`): sent as `CASE WHEN e IS NULL THEN 1 / CASE WHEN e IS NULL THEN 0 ELSE 1 END ELSE e END`, and AGE's "division by zero" from such a statement is answered with that error (`cypher.rs` `guard_merge_nulls`, `mod.rs` `null_merge_refusal`) |
| an aggregation next to a list comprehension | three projections: lists, aggregates, the rest |
| variables named `id`, `properties`, `start_id`, `end_id` | renamed (they clash with the label tables' columns) |
| a leading `OPTIONAL MATCH` | `WITH 1 AS …` in front (AGE returns no row when nothing matches) |
| `ORDER BY m.k` of a projected map `m` | the key read from where the map got it |
| `RETURN *` | the variables in scope, sorted |
| `FOREACH`, `CALL { … }`, `SET n:L` / `REMOVE n:L`, a SET after a MERGE that has ON CREATE/ON MATCH items | cut into several statements in one transaction, the rows carried between them as a parameter (vertices and edges by id) — AGE has no FOREACH or CALL {}, keeps one label per vertex, and drops a SET that follows such a MERGE |
| `CALL { … UNION [ALL] … }` (SUITES B2: "could not find rte for s" — the rows were brought in on the first branch only) | every branch gets them: `UNWIND $__og_rows AS __og_row OPTIONAL MATCH (s:L) WHERE id(s) = __og_row.s WITH __og_row, s <branch> UNION ALL UNWIND $__og_rows …` |
| `CALL { … }` names (SUITES B3: answered where Neo4j refuses) | checked before anything runs, with Neo4j's `SyntaxError` and wording: a name it returns that is bound outside ("Variable `s` already declared in outer scope"), a name it imports — `CALL (nope)`, a leading `WITH nope` — that is not bound ("Variable `nope` not defined") |
| a FOREACH, `CALL { … }` or label change inside a FOREACH or `CALL { … }` body (SUITES B4: refused) | the body cut into steps of its own and run on the rows the outer step gives it (`split.rs` `plan_in` / `run_on`): a FOREACH's on one row per row and element, made by one statement (`… UNWIND list AS x RETURN <scope>, x`), a CALL's once per row. A vertex several such rows hold changes table once |
| a WHERE after a SET / REMOVE whose part reads only what was there at the SET (SUITES N4: AGE evaluates it below the SET — `MATCH (s:V) SET s.v = s.k * 10 WITH s WHERE s.v > 10` filters on the old value, and the rows it drops are never written) | cut before the WHERE's clause (`split.rs` `Cut`); a WHERE on what came after the SET (`OPTIONAL MATCH (a …) WHERE a IS NOT NULL`, an UNWIND's variable) stays |
| a MATCH / OPTIONAL MATCH after a CREATE / MERGE in the same part whose new elements can see what was written — a label or type written there, a node without a label, a relationship without a type (SUITES N3: `UNWIND [1,2,3] AS i CREATE (:V {k: i}) WITH count(*) AS made MATCH (t:V)` sees 1 of 3) | cut before the MATCH. `MERGE (a:X …) WITH a MATCH (b:Y …)` stays one statement |
| a MERGE whose ON CREATE / ON MATCH items read what it merges (SUITES N1, and #7 for a relationship: "attribute 1 of type record has wrong type", "binary container must be a VLE vpc") | the MERGE with `ON CREATE SET m.__og_c = true` on its first relationship (given a variable) or its node; then the ON MATCH items on the rows that did not create (`NOT first-to-reach OR m.__og_c IS NULL`), the ON CREATE items and `m.__og_c = null` on those that did. Items that read only other variables leave the MERGE as it is |
| a step that writes (SET / REMOVE first in a part, or N1's items) on rows holding the same element more than once (SUITES N2: `UNWIND [1,1,1] AS i MERGE (t:U {k: i}) ON MATCH SET t.hits = coalesce(t.hits, 0) + 1` stores 1; each row's SET starts from the element as it was before the statement, and a final SET skips a tuple already updated in its command) | run in waves — no element twice in a wave, each element's rows in order (`split.rs` `waves`); one statement when the rows' elements are distinct. The rows the statement returns afterwards show the final values, where Neo4j's would show each row's own |
| `CREATE CONSTRAINT … IS UNIQUE`, `CREATE INDEX` | unique / plain expression indexes on the label's table (made if missing); `IS NOT NULL`, full-text and vector are accepted and not enforced |
| `SHOW GRAPHS`, `CREATE GRAPH g` | answered from `ag_catalog` |
| procedures and `SHOW INDEXES` / `SHOW CONSTRAINTS` (SUITES B5–B7, STUDIO G5a–G5d, ANALYZER §5.2: refused, "CALL x() inside a query is not supported") | answered by the gateway (*Procedures and SHOW* below): alone, from the catalog; with `YIELD … WITH … WHERE … RETURN … ORDER BY` after it, the rows as a parameter and the rest in Cypher — `UNWIND $__og_proc1 AS label …` |
| `CALL apoc.neighbors.tohop(n, '>', d) YIELD node AS m` (STUDIO G5c) | `MATCH (n)-[*1..d]->(m) WHERE id(m) <> id(n) WITH DISTINCT …, m`, `d` a literal or a parameter; with no type or several and over 3 hops, walked by `reach.rs` (the MATCH is followed by DISTINCT) |
| an unknown procedure (SUITES B12: was `SyntaxError`) | `Neo.ClientError.Procedure.ProcedureNotFound`, Neo4j's wording |
| a variable-length relationship without exactly one type over 3 hops (`-[:A\|B\|…*1..30]-`, `<-[*1..30]-`) where only the nodes it reaches matter: in `EXISTS { }` between two bound nodes, or as the only pattern of a MATCH followed by `DISTINCT`, `DISTINCT` aggregates (and `min`/`max`) or `LIMIT 1` | walked by the gateway (`reach.rs`): the start nodes' ids from the clauses before it, a breadth-first walk with one SQL statement per hop on the label tables' `start_id`/`end_id`, then the statement with the pattern replaced by `id(t) IN $reached[id(s)]`. Neo4j's trail semantics: the start node counts as reached only over a closed trail (a cycle through it) within the bound |

### Procedures and SHOW

`src/age/procs.rs`. What a client asks of a procedure here is the graph's
schema, which AGE keeps in `ag_label` and the gateway's schema commands keep
in PostgreSQL's catalog (the indexes `ddl.rs` makes on the label tables) —
catalog reads, DIRECTION §4.2 rule 6. A statement that is only the call, or
the call and a `RETURN` of what it yields, is answered from those rows; in
any other statement the call becomes `UNWIND $__og_procN AS label` (one
column yielded) or `UNWIND $__og_procN AS __og_procrowN WITH <what is in
scope>, __og_procrowN.name AS name, …`, with the rows read into
`$__og_procN` first, and everything after it is Cypher as written. A
statement that ends with the call (or with `SHOW … YIELD … WHERE …`) returns
what it yields. A `YIELD` of a column the procedure does not have is
`SyntaxError` "Unknown procedure output".

| Procedure | Answer |
|---|---|
| `db.labels()`, `db.relationshipTypes()` | `ag_label`, kind `v` / `e`, AGE's two base tables left out |
| `db.propertyKeys()` | `keys()` of every node and relationship, read through Cypher (two statements; it was a SQL read of the label tables) |
| `SHOW INDEXES` | every index on a label table whose key columns are property accesses (what `ddl.rs` makes; AGE's own on `id`, `start_id`, `end_id` are plain columns and left out), from `pg_index` / `pg_get_indexdef`, under the name `ddl.rs` gave it. Columns as Neo4j 5 (and 4.4's `uniqueness`) name them: `id` (the index's oid), `name`, `state` (`ONLINE`; `FAILED` for an index PostgreSQL holds invalid), `populationPercent` 100.0, `uniqueness`, `type` `RANGE`, `entityType`, `labelsOrTypes`, `properties`, `indexProvider` `range-1.0`, `owningConstraint` (the constraint's name for its unique index), `lastRead` / `readCount` / `trackedSince` null, `options` `{indexProvider: 'range-1.0', indexConfig: {}}`, `failureMessage`, `createStatement` |
| `SHOW CONSTRAINTS` | the unique ones: `type` `UNIQUENESS` (`RELATIONSHIP_UNIQUENESS`), `ownedIndex` = `name`, `propertyType` null, `options`, `createStatement`. A `NODE KEY` constraint reads back as `UNIQUENESS` (`ddl.rs` makes a unique index for it) |
| `dbms.components()` | one row: `Neo4j Kernel`, `['4.4.0']`, `community` — what `HELLO`'s agent says |
| `db.awaitIndexes(n)`, `db.awaitIndex(name, n)` | nothing: `ddl.rs` makes an index within the statement. Alone, no rows; in a statement, the rows go on unchanged. `awaitIndex` does not check that the index exists |
| `apoc.meta.schema()` | one row, `value`: per vertex label `{type: 'node', count, labels, properties, relationships: {}}`, per type `{type: 'relationship', count, properties}`, each property `{type, array, indexed, unique, existence: false}`. Approximate: a property's type is the one seen most in a sample of 100 elements stored under each label (`MATCH (n:L) WHERE label(n) = 'L' WITH n LIMIT 100 RETURN properties(n)`; a label's properties are its own table's and those of the tables inheriting it); dates are text here and read `STRING`; a node label's `relationships` is left empty; `labels` are the ones its table inherits. `count` is exact (`MATCH (n) RETURN label(n), count(*)`, summed down the hierarchy) |
| `apoc.neighbors.tohop(n, filter, d)` | the rewrite in the table above. The filter is APOC's (`'>'`, `'<KNOWS'`, `'KNOWS>|LIKES>'`, `''` / null for both ways), one direction for all its types; `d` from 1 to 30. Two rows the same in every variable before the call come out as one (the DISTINCT), where APOC is called for each |

There are no vector or full-text index rows: `CREATE VECTOR INDEX` /
`FULLTEXT` are accepted and not made, and nothing records them. So the
analyzer's wait for `analyzer_search_vec` to be `ONLINE`
(`embed_step.py` `_await_index_online`) sees no row and times out — the
embedding step cannot finish on this backend until vector search has a home
(it is not the gateway's: DIRECTION §4.2 rule 7). `db.index.vector.queryNodes`,
`db.index.fulltext.queryNodes` and every other procedure are
`Neo.ClientError.Procedure.ProcedureNotFound`, the message saying which ones
the backend answers.

Refused by name: such a relationship where the trails themselves matter (a
relationship or path variable on it, `count(*)` or rows per trail after it, a
lower bound above 1, two of them in one statement — AGE enumerates every
trail; `*1..30` over ten types ran the server out of memory), a choice of
labels (`:A|B`) or of relationship types in `CREATE` / `MERGE`, the label
writes *Label hierarchy* lists, and Ontological's fabric statements.

Error codes (`mod.rs` `error_code`; SUITES B10, B12): AGE's type errors are
answered as `Neo.ClientError.Statement.TypeError`, as Neo4j answers them —
22023 "Invalid input parameter types for agtype_mul" (an operator over a value
it does not take, `sum(m.name * 2)`; the gateway had read the word
"parameter" in it as `ParameterMissing`), 0A000 "… clause can only update
vertex and edges" / "can only delete vertices and edges" (a SET or DELETE of a
map or a scalar; was `SyntaxError`) and XX000 "container is not an agtype
array" (N8, `WITH {a: 1} AS n SET n.x = 1`). A missing parameter map (42P02)
stays `ParameterMissing`; a key missing from the map reads as null in AGE.

`summary.counters` come from PostgreSQL's per-transaction statistics of the
graph's label tables (`pg_stat_xact_*`), read before and after the statement:
nodes and relationships created and deleted are exact; `properties-set`
counts updated elements, not properties; `labels-added` counts one label per
node created (its table's — not the ones it has through inheritance, where
Neo4j counts every label of `(:A:B:C)`). A write sent without `BEGIN` runs in
a transaction of its own for that reason.

### Label hierarchy

A vertex in AGE is a row of one label table, so it has one label. The nodes
Studio and the analyzer write have several — an umbrella and a kind,
`(:_Entity:Customer)`, `(:ANALYZER_OWNED:TABLE)` — and are found by either.
Here the kind's table **inherits** the umbrella's (PostgreSQL's `ALTER TABLE …
INHERIT`), and a node has label L when its table is L's or inherits L's,
directly or not (`src/age/hierarchy.rs`). AGE's label scan takes the
inheriting tables along, so `MATCH (n:_Entity)` finds the Customers with
nothing rewritten — a single-label MATCH is sent as it is.

- **Storage.** `CREATE` / `MERGE (n:A:B:C {…})` stores the node under the last
  label, C (the specific one, as both clients write them), unless one of the
  labels given inherits all the others already, which is then used. Before the
  statement, in its transaction, the gateway makes the label tables that are
  missing (`SELECT ag_catalog.create_vlabel('<graph>'::cstring,
  '<label>'::cstring) WHERE NOT EXISTS (… ag_label …)`) and gives C's table
  what it does not inherit yet (`ALTER TABLE "<graph>"."C" INHERIT
  "<graph>"."B"`, the nearest label first, so what B inherits is not added
  again). Only C's table changes: B does not become an A. The data itself is
  written by AGE's Cypher, `CREATE (n:C {…})`.
- **Refused** (`Neo.ClientError.Statement.SemanticError`): C exists and holds
  nodes — in its table or one inheriting it, asked as `MATCH (n:C) RETURN
  id(n) LIMIT 1` before anything is made — and does not inherit A yet:
  inheriting would give every existing C node label A unasked ("… C already
  has nodes, and they would all become A as well. Use a label that has no
  nodes yet, or delete the C nodes first"); a cycle (C would have to inherit A,
  whose table inherits C's); a label that is a relationship type in the graph.
- **Reads** with more than one label, `n:L` and `labels()` are printed from
  the hierarchy (rows above). It is read from the catalog — `ag_label` joined
  with `pg_inherits` — once for each statement that needs it (a node pattern
  with more than one label, `n:L`, `labels()`, a label SET / REMOVE), and kept
  for nothing else; a plain `MATCH (n:L)` does not read it:

  ```sql
  SELECT l.name::text, l.kind::text, p.name::text
  FROM ag_catalog.ag_label l
  JOIN ag_catalog.ag_graph g ON g.graphid = l.graph
  LEFT JOIN pg_catalog.pg_inherits i ON i.inhrelid = l.relation::oid
  LEFT JOIN ag_catalog.ag_label p ON p.relation::oid = i.inhparent AND p.graph = l.graph
    AND p.kind = 'v' AND p.name <> '_ag_label_vertex'
  WHERE g.name = '<graph>' AND l.name NOT IN ('_ag_label_vertex', '_ag_label_edge')
  ORDER BY 1, i.inhseqno
  ```
- **Label changes** (`split.rs`). `SET n:L` of a label the node has through its
  table does nothing (no `labels-added`); `REMOVE n:L` of such a label, while
  the table's own label stays, is refused ("`n` is stored as Customer, and
  Customer's table inherits _Entity — every Customer node is _Entity …").
  A vertex moved to another table — `REMOVE n:Customer SET n:Client`, its row
  moved as before, the one place the gateway writes a label table — has the
  new table inherit the old one's parents first (DDL only; refused as above
  when the new table exists and holds nodes). Removing a node's own label
  moves it to its parent's table; setting a label below its own moves it down.
- **The id changes** when a vertex changes table: AGE's graphid carries the
  label's number. An element id a client kept from before the change (a screen
  showing the node) no longer finds it. That is AGE's limit; only a changed AGE
  would keep it.
- **Uniqueness is per table.** `CREATE CONSTRAINT … FOR (n:ANALYZER_OWNED)
  REQUIRE n.id IS UNIQUE` makes a unique index on `ANALYZER_OWNED`'s table and
  answers as before (`constraints-added: 1`), but PostgreSQL indexes are not
  inherited: it does not cover the nodes stored in `TABLE`, `COLUMN`, … whose
  tables inherit it, and two of those may share an id. The gateway does not
  emulate a constraint across tables. A constraint on the label a node is
  stored under (`FOR (n:TABLE)`) holds; the analyzer's `MERGE` on the specific
  label is what keeps one node per key.
- **Bolt nodes.** A node returned as a value (`RETURN n`) carries its table's
  one label in `Node.labels`; `labels(n)` gives the inherited ones as well.
- That AGE's scans, `MERGE`, `DELETE`, `label()` and `drop_label` follow
  PostgreSQL inheritance is AGE 1.8.0's observed behaviour (STUDIO-ON-AGE
  §2.5), not a documented one. The probe tests (`tests/age-probes/`) are to
  check it on every AGE version.

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
