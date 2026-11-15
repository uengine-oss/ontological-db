# Cypher reference

What this release supports, precisely — and what it does not. Unsupported syntax
fails at parse or compile time with the construct named and an alternative
suggested. It is never silently reinterpreted.

```sql
SELECT og_cypher(graph, query, params);        -- SETOF jsonb, one object per row
SELECT og_cypher_json(graph, query, params);   -- one jsonb array (PostgREST/RPC)
SELECT og_cypher_sql(graph, query);            -- the compiled SQL
SELECT og_cypher_explain(graph, query, true);  -- compiled SQL + PostgreSQL plan
SELECT og_cypher_check(query);                 -- parse only
```

---

## Supported

### Reading

| Construct | Notes |
|---|---|
| `MATCH` | multiple patterns, comma-separated |
| `OPTIONAL MATCH` | compiles to `LEFT JOIN LATERAL` |
| `WHERE` | full expression language below |
| `RETURN` | with `DISTINCT`, aliases, `*` |
| `ORDER BY` | ascending/descending; may reference a `RETURN` alias |
| `SKIP` / `LIMIT` | expressions allowed |
| `UNWIND … AS x` | over a list expression |

### Patterns

```cypher
(a)                          -- any node
(a:Person)                   -- label; matches every subtype
(a:Person {name: 'Aria'})    -- inline property filter
(a)-[r:KNOWS]->(b)           -- directed, typed, bound
(a)<-[:KNOWS]-(b)            -- reverse
(a)-[:KNOWS]-(b)             -- either direction
(a)-[:KNOWS|FOLLOWS]->(b)    -- type alternatives
(a)-[:KNOWS*1..3]->(b)       -- variable length, trail semantics
p = (a)-[:KNOWS]->(b)        -- path variable
```

Over a **virtual relationship** (a join on declared keys), an end without a
label takes its type from the relationship: `(o:Order)-[:PLACED_BY]->(c)`,
`(c)-[:PLACED_BY]-(o:Order)` (undirected: the labelled end decides) and a node
carried through `WITH` (`MATCH (c:Customer) WITH c MATCH (c)<-[:PLACED_BY]-(o)`)
all work. Only an undirected hop with no label on either end is refused.

**A hop without a type** — `(c:Crate)-[r]->(d:Depot)`, `(c:Crate)-[r]-(d:Depot)`,
`(c:Crate)-->(d:Depot)` — is every relationship between its ends, as in Neo4j: the union of
each virtual relationship whose ends fit the two labels and the direction, and
the stored edges. It returns the rows the typed hops return together, and
`type(r)` is each row's own type. `[r:A|B]` is the union of `A` and `B`; when
every type named is virtual and they agree on an unlabelled end, that end takes
the type, as for one type. The ends' types are what is joined on, so:

- with one end unlabelled, a virtual node at the other end is refused ("needs
  both ends labelled", naming its relationships) — label the end or name the
  type; at a stored node the hop reads its stored edges only, as before, with
  a note naming the virtual relationships it did not follow;
- a variable-length hop without a type follows stored edges only, and is
  refused at a virtual node that has virtual relationships;
- a property filter on the hop (`-[r {x: 1}]->`) matches stored edges only —
  a virtual relationship has no properties.

It is one join on (alternative, key) — each end lists its key for every
alternative — so it costs a few times the typed hop, not a join per type; the
typed hop stays the one a source can be sent whole. Under `OPTIONAL MATCH` the
near end is read again and matched back on its id, so an optional untyped hop
reads both ends' types in full even from one bound node. Between two nodes of
one type, the undirected form reads each relationship once, as the typed hop
does. A pattern with no label at all reads the stored graph, as `MATCH (n)` does.

On a virtual type whose source has a column `id`, `n.id` is that column (so
`{id: 1}` and a relationship joining on it work); `id(n)` and `n._id` are the
node's identifier. On stored types `n.id` is the identifier, as before.

A label matches the type **and all of its subtypes**. That is resolved once at
compile time through the interval index, so it costs nothing per row.
A label over very many types — an umbrella supertype over every class of an
application — reads them through one generated view. Past 200 distinct property
columns that view keeps as columns only the properties two or more of its types
share; a property only one type has is read from the row's JSON instead, still
typed as declared (`n.qty + 1`, `sum(n.qty)` work) but without an index. Any
number of types and properties works; `MATCH (n:Type)` on one type is unchanged.

Variable-length paths never repeat a relationship within one path, which is what
keeps a cycle from expanding forever. `*..` without an upper bound is capped at 8
hops.

Relationships match isomorphically, as in Cypher: one `MATCH` clause never
traverses the same relationship twice. That is what stops
`(a)-[:ACTED_IN]->(m)<-[:ACTED_IN]-(b)` from returning `a` as their own
co-actor. The rule is scoped to the clause, so two separate `MATCH` clauses may
still bind the same relationship.

### Writing

| Construct | Notes |
|---|---|
| `CREATE` | nodes and relationships; exactly one label/type per new element |
| `MERGE` | match-or-create, with `ON CREATE SET` / `ON MATCH SET` |
| `SET` | `a.prop = expr`, `a = {…}`, `a += {…}` |
| `REMOVE` | `a.prop` |
| `DELETE` / `DETACH DELETE` | plain `DELETE` refuses a node that still has relationships |

Write clauses run over the bindings the read part produced, inside the caller's
transaction. Adjacency is maintained in both directions in the same transaction.

### Expressions

```
literals      42   3.14   'text'   "text"   true   false   null   [1,2]   {k: v}
parameters    $name                                (bound, never interpolated)
properties    a.name    r.since
comparison    =  <>  !=  <  <=  >  >=   IS NULL   IS NOT NULL   IN
string        STARTS WITH   ENDS WITH   CONTAINS   =~
boolean       AND  OR  XOR  NOT
labels        n:A   n:A:B   n:A|B   NOT n:A        (a predicate, in WHERE or RETURN)
arithmetic    +  -  *  /  %  ^  ||
lists         xs[0]   xs[-1]   xs[1..3]   xs[..2]   xs[2..]   [1,2] + [3]
conditional   CASE … WHEN … THEN … ELSE … END
```

**Arithmetic on values of unknown type.** A value carried through `WITH` or
`UNWIND`, or an undeclared property, is read as a number where arithmetic or
`sum`/`avg` needs one, so `WITH o RETURN sum(o.qty * o.price)` and
`WITH o.qty * o.price AS v RETURN sum(v)` both work. Integers divide as integers
(`7 / 2` is `3`), anything else exactly. A node carried through `WITH` keeps its
type, so its properties keep theirs. `+` with a string concatenates, with a
list appends. `min`/`max` of a value of unknown type order numbers by value and
strings as text.

**A boolean against 0 or 1.** Sources without a boolean type keep flags as 0/1,
so `n.active = 1` means `n.active = true` (and `= 0`, `IN [0, 1]` likewise); a
boolean compared with any other number reads as 1 or 0 (`n.active = 2` is never
true), and a 0/1 number column compared with `true` is compared with 1. A
literal 0 or 1 becomes the boolean itself, so the comparison still reaches the
source as `active = true`.

**A label predicate** is decided against the type hierarchy at compile time —
`n:A` holds for `A` and its subtypes, as `MATCH (n:A)` does — so over a virtual
node it folds to a constant. An umbrella label beside a virtual type
(`n:_Entity` on a virtual `Customer`) holds, as it does in `MATCH`.

**`id` is an ordinary property**, as in Neo4j: `CREATE (n:Lamp {id: 'singleton'})`
then `MATCH (n:Lamp {id: $id})` and `n.id` read the property you stored (null where
there is none). The node's identifier is `id(n)`, `elementId(n)` and `n._id`. (Before
this, `n.id` on a stored type was the identifier, and `{id: 'x'}` failed with
"invalid input syntax for type bigint".)

**Unlabelled patterns stay in the session's graph.** `MATCH (n)` and
`MATCH (a)-[r]->(b)` read the graph the session is on, not every graph in the
database; so `MATCH (n) DETACH DELETE n` empties one graph. Writes are confined
the same way: a map carrying an `_id` (`WITH $m AS n SET n.x = 1`, `$m = {_id: …}`)
reaches only an element of this graph — one of another graph is refused by `SET`,
`CREATE`, `MERGE` and `DELETE` ("is not a node or relationship of this graph"), and
matches nothing in a pattern.

**Pattern predicates and subqueries** (Neo4j 5): `WHERE EXISTS { (a)-[:R]->(:B) }`,
`EXISTS { MATCH … WHERE … }`, `exists((a)-[:R]->())`, a bare `WHERE NOT (a)-[:R]->()`,
`COUNT { pattern }`, `COLLECT { MATCH … RETURN x }` and pattern comprehensions
`[(a)-[:R]->(b) WHERE … | b.name]` are compiled to correlated sub-selects, so they
neither add rows nor leak bindings. They are values for a read; in `SET`/`CREATE` they
are refused ("cannot be evaluated in a write clause") — compute them in a `WITH` first.
`FOREACH (x IN list | SET … / CREATE … / MERGE … / nested FOREACH)` runs its write
clauses once per element (a read inside it is refused). `ORDER BY alias.prop` sorts
by a property of a projected map or node.

**Writes.** `UNWIND $rows AS item MERGE (n:L {k: item.k})` matches existing nodes
(a null key is refused, as Neo4j refuses it); `WITH x AS alias`, or any expression,
may precede a write clause, and a reading clause after a write runs for each row.

**Subqueries: `CALL { … }`.** `CALL { WITH a, b … }` imports `a` and `b` (a leading
`WITH` of plain variables, or `WITH *`); the Neo4j 5 scope clause `CALL (a, b) { … }` /
`CALL (*) { … }` does the same. Only imported variables are visible inside, and a
subquery may not return a name already defined outside it. It runs once per incoming
row:

- one that only reads is a correlated `LATERAL` join — each row it returns joins its
  outer row, none drops the row; `ORDER BY … LIMIT` inside is per row; an aggregate
  inside returns a row over nothing (`count()` is 0);
- one that writes runs its body row by row, in order, each run seeing what the runs
  before it wrote; a body without `RETURN` (a unit subquery) leaves the rows as they
  were, and what it did to an imported node is what the query sees after it;
- `OPTIONAL CALL { … }` keeps a row the body has nothing for, with nulls;
- `UNION` / `UNION ALL` inside combine branches that return the same columns; nested
  subqueries work;
- `CALL { … } IN TRANSACTIONS` is refused: a statement here runs in one transaction,
  so send batches as separate statements (or drop `IN TRANSACTIONS`).

**Null in a list is null.** An element of an `UNWIND`ed list, of a comprehension's
list, or a value carried across a write that is `null` counts as null —
`count(x)` skips it, `x IS NULL` holds.

**Parameters over Bolt.** A date, time, datetime or duration parameter is the ISO 8601
text Neo4j prints it as (`date($d)` and comparisons with a date column work); a point
parameter is refused by name. A path result is a Bolt `Path`. A map parameter's
properties read: `$row.name`, `WITH $row AS r RETURN r.name`.

### Functions

| Group | Functions |
|---|---|
| aggregate | `count` (incl. `DISTINCT` and `count(*)`), `sum`, `avg`, `min`, `max`, `collect`, `stdev` |
| list | `head`, `last`, `tail`, slices `xs[a..b]` (from inclusive, to exclusive, negative from the end) |
| graph | `id`, `elementId`, `labels`, `type`, `nodes`, `relationships`, `length`, `size` (a string's characters, a list's items, `null` for `null`; a map has none and is refused), `startNode`, `endNode` |
| string | `toUpper`, `toLower`, `trim`, `ltrim`, `rtrim`, `left`, `right`, `substring`, `replace`, `split` |
| numeric | `abs`, `ceil`, `floor`, `round`, `sqrt`, `rand`, `sign` (an integer) |
| conversion | `toString`, `toInteger`, `toFloat` — a number truncates toward zero, a string that is not a number gives `null` |
| temporal | `date()`, `date('2016-01-31')`, `date({year, month, day})`, `datetime()`, `datetime('…')`, `localdatetime(…)`, `duration('P30D')`, `duration({days: 30})`, `timestamp()`; fields `.year` `.quarter` `.month` `.week` `.day` `.ordinalDay` `.dayOfWeek` `.hour` `.minute` `.second` `.millisecond` `.epochSeconds` `.epochMillis` |
| list / other | `reduce(acc = init, x IN xs \| e)`, `range(from, to[, step])` (both ends included), `reverse` (a list or a string), `coalesce` (keeps a list a list), `nullIf(v, w)`, `isEmpty` (a list, a map or a string), `toBoolean`, `keys` (an element's properties only), `properties`, `exists` (a property, or a pattern: `exists((a)-[:R]->())`), `randomUUID()`, `vector.similarity.cosine(a, b)`, `vector.similarity.euclidean(a, b)` |
| **vector** | `vector.similarity(a, b)`, `vector.distance(a, b)`, `vector.l2(a, b)`, `genai.vector.encode(text, provider, config)` |

`vector.similarity` compiles to `1 - (a <=> b)` on a pgvector column, so it uses
the HNSW index when the query is shaped for it.

A date literal is converted when the statement is planned, so on a virtual node
`o.placed >= date('2016-01-01')` reaches the source as `placed >= '2016-01-01'`.
`date()` (today) and field access (`o.placed.year = 2016`) are evaluated here,
after the rows arrive: postgres_fdw sends neither a non-immutable function nor
`extract()`, whose field argument carries a collation. A date moved by a
duration stays a date. `datetime` is a `timestamptz`, so its fields are read in
the session's time zone.

### Statement time limit

Every Cypher statement — and every fabric statement — runs under
`og.statement_timeout`, two minutes unless set. When it runs out the statement is
cancelled the way PostgreSQL's `statement_timeout` cancels one (SQLSTATE 57014,
"canceling statement due to statement timeout"), and on virtual nodes
postgres_fdw cancels the source's half too. A stricter `statement_timeout` is
not loosened.

```sql
SET og.statement_timeout = '30s';             -- 30000, '2min'; 0 = no limit
ALTER DATABASE og SET og.statement_timeout = '5min';
SELECT og_statement_timeout();                -- the limit now, in ms
```

Over Bolt a driver's transaction timeout (`Query(…, timeout=…)`,
`begin_transaction(timeout=…)`) sets it for that transaction, and the error is
`Neo.ClientError.Transaction.TransactionTimedOut`; see `bolt/README.md`.

### SQL over the sources: `QUERY FABRIC`

A fabric statement, sent wherever Cypher is (`og_fabric`, `og_cypher`, Bolt), for
a question that is a table question:

```sql
QUERY FABRIC SELECT s.name, count(*) FROM erp.supplier s JOIN erp.purchase_order po USING (supplier_id) GROUP BY 1
QUERY FABRIC WITH {datasource: $ds, max_rows: 1000, max_bytes: 67108864} SELECT * FROM supplier WHERE rating > 4
```

One `SELECT`/`WITH` over the datasources' tables, read only, nothing more. It
runs as `og_source_reader` (no login; `SELECT` on the datasources' foreign
tables, granted as `CREATE`/`REFRESH DATASOURCE` bring them in) in a read-only
subtransaction that is always rolled back; any other relation — a local table,
`og_catalog`, `og_data`, `pg_catalog`, `information_schema` — and any `pg_*` or
`og_*` function is refused (42501), also inside a nested query. Unqualified names
find the named datasource's tables, or any datasource's. The time limit is the
statement's own (`og.statement_timeout`; over Bolt, the driver's timeout). The
answer is one row: `columns` and `types` (lists, in order), `rows` (lists of typed
values — numbers, booleans, JSON; an `int8`/`numeric` beyond 15 digits and
everything else as PostgreSQL prints it), `row_count`, `truncated` (more rows than
`max_rows`, default 1000), `bytes`, `elapsed_ms`; over `max_bytes` (default 64 MB) it
is refused (54000). Every error message ends in its SQLSTATE —
`column "nope" does not exist [SQLSTATE 42703, position 8]`, the position counted
in the SQL. Only a superuser or a member of `og_source_reader` may run it. Writing,
and mixing SQL into a graph pattern, stay with virtual nodes
(`CREATE VIRTUAL NODE … AS SELECT …`).

### Parameters

```sql
SELECT og_cypher('kb',
  $$ MATCH (p:Person) WHERE p.age > $min AND p.city = $city RETURN p.name $$,
  '{"min": 30, "city": "Seoul"}');
```

A parameter is cast to the declared type of the column it is compared against,
so the index stays usable. A parameter can never change the structure of a
query, which makes injection structurally impossible rather than filtered.

---

## The Neo4j surface

Cypher written for Neo4j does not stop at the query language. Applications
create indexes by name and then query them by that name, call `db.*`
procedures, and reach for a few APOC helpers. All of that is here, **under the
original names**, so an application moves by changing its URI.

### Clauses

| Construct | Notes |
|---|---|
| `WITH` | the query horizon: aggregation, then `WHERE` on the grouped rows, then the next segment. `DISTINCT`, `ORDER BY`, `SKIP`, `LIMIT` all apply |
| `CALL … YIELD …` | the procedures below; yielded columns bind like anything else |
| `[x IN xs WHERE p \| e]` | list comprehension, both halves optional |
| `any` / `all` / `none` / `single` `(x IN xs WHERE p)` | list predicates |
| multi-label patterns `(a:A:B)` | the most specific label decides; see "Differences". Beside a single virtual type, a label naming a supertype it is not placed under (an umbrella such as `_Entity`) is set aside, with a note |
| `CREATE`/`DROP INDEX`, `CREATE`/`DROP CONSTRAINT` | see below |

### Procedures

| Procedure | Reaches |
|---|---|
| `db.index.vector.queryNodes(name, k, vector)` | `og_vector_search` |
| `db.index.fulltext.queryNodes(name, query)` | PostgreSQL full-text search — **not equivalent**, see below |
| `apoc.neighbors.tohop(node, relFilter, hops)` | the variable-length walk (`og_vlp`) |
| `apoc.meta.schema({sample: n})` | the type catalog, in APOC's shape — `og_apoc_meta_schema` |
| `db.labels()`, `db.relationshipTypes()`, `db.propertyKeys()` | the type catalog |
| `dbms.components()` | version reporting |
| `db.awaitIndex`, `db.awaitIndexes`, `db.clearQueryCaches` | accepted, do nothing — indexes here are built synchronously |

The registry is closed: any other procedure is refused **by name**. An
application calling something unsupported is told so rather than handed an empty
result it cannot tell from "no matches".

`apoc.meta.schema` is the one that most Neo4j tooling reaches for first, because
Neo4j has no declared schema to read and APOC has to *sample* the store to guess
one. Here the schema is declared, so mostly nothing is sampled: `count` is a
count rather than an estimate, property types are the ones the catalog holds,
and relationship direction comes from the declared roles instead of from
whichever direction the sample happened to contain.

The exception is a relationship type created the Neo4j way — by writing
`(a)-[:LINKS]->(b)` without declaring it. That type has no roles, so its
endpoints are read from its edges, bounded by `sample` (default 1000). Those
pairs are APOC's kind of answer and carry APOC's caveat: they describe what the
sample contained, not a constraint the database will enforce.

### `genai.vector.encode`

Vector search takes a vector; a question is a string. `genai.vector.encode` is
Neo4j's answer to that gap and this is the same function under the same name, so
the resolution is one statement:

```cypher
CALL db.index.vector.queryNodes('room_name', 3, genai.vector.encode($text))
YIELD node, score RETURN node.name, score
```

Which matters more than it looks: a client with only Cypher — an MCP server, a
BI tool, a driver — has no way to produce an embedding, so keeping the step
inside the query is what keeps semantic search reachable from one at all. It
also means the stored vectors and the query vector come from the same
configuration by construction, rather than by two codebases agreeing.

It is **off by default**, because it makes a PostgreSQL backend block on an
external HTTP server:

```sql
SELECT og_set_setting('genai.enabled',    'on');
SELECT og_set_setting('genai.endpoint',   'http://localhost:11434/api/embed');
SELECT og_set_setting('genai.provider',   'ollama');   -- or OpenAI-compatible
SELECT og_set_setting('genai.model',      'qwen3-embedding:latest');
SELECT og_set_setting('genai.dimensions', '1024');     -- truncate, then re-normalise
SELECT og_set_setting('genai.token',      '…');        -- optional bearer token
SELECT og_set_setting('genai.timeout_ms', '5000');
```

**The endpoint is configuration, never an argument** — the one place this
deliberately departs from Neo4j, which lets the call name its own. Here a caller
who can write Cypher cannot make the server fetch a URL of their choosing:
query rights are not fetch rights.

`dimensions` truncates and re-normalises. That is sound for a Matryoshka-trained
model, whose prefix is a valid smaller embedding, and it is necessary for one
wider than 2000 dimensions because that is where pgvector's HNSW index stops.

### `EXPLAIN` and `PROFILE`

Both prefixes are accepted. The query is parsed and classified but **not run**,
and the result is empty with the summary's `type` set to `r` or `w`. That field
is how driver-side tooling tells a read from a write before executing it — the
Neo4j MCP server gates both of its query tools on it — so it is answered from
the parser rather than from a keyword scan.

No plan is returned. Neo4j's `EXPLAIN` describes its own operators, and inventing
an equivalent for a query that becomes ordinary SQL would be fiction; use
`og_cypher_sql()` for the statement and `EXPLAIN` it as SQL, or
`og_cypher_explain()` for both at once. `PROFILE` is treated as `EXPLAIN` for
the same reason.

An index name must be a literal string — it is resolved when the query is
compiled, so a parameter cannot name one.

### Index and constraint DDL

`CREATE INDEX`, `CREATE TEXT|RANGE|POINT INDEX`, `CREATE VECTOR INDEX`,
`CREATE FULLTEXT INDEX`, `CREATE CONSTRAINT … REQUIRE … IS UNIQUE | IS NOT NULL
| IS NODE KEY` (and Neo4j 4's `ASSERT`), with `IF NOT EXISTS`; `DROP INDEX` /
`DROP CONSTRAINT` with `IF EXISTS`.

The name is recorded in `og_catalog.compat_index` and is what
`db.index.*.queryNodes` resolves. A property that has not been declared is
declared on the way — including moving values already written under it into the
new column, because "write first, index later" is the ordinary order.

`DROP INDEX` forgets the name; it does not drop the property or its data.

---

## Not supported in this release

| Construct | What happens | Alternative |
|---|---|---|
| `UNION` in a query that writes | refused | run the branches as separate statements (`UNION` inside `CALL { … }` works) |
| `CALL { … } IN TRANSACTIONS` | refused at parse time | separate statements per batch, or the subquery without `IN TRANSACTIONS` |
| spatial values — `point()`, `point.distance()`, point parameters | refused by name | keep coordinates as numbers |
| `SET a:Label` adding a *new* label | rejected with the reason | a node's type is part of its identifier here. To rename a class write `REMOVE n:Old SET n:New`; to move a node between types, create it under the target type |
| `shortestPath` | not implemented | `og_vlp()` ordered by depth |
| user-defined procedures | no mechanism | the `og_*` SQL function surface |

Check the current state programmatically:

```sql
SELECT og_cypher_check('MATCH (a) WITH a RETURN a');
-- {"ok": true, "clauses": 3, "write": false}
SELECT og_explain_error('kb', 'MATCH (a) RETURN nosuchfunction(a)');
-- {"ok": false, "code": "UNSUPPORTED_SYNTAX", "message": "unknown function …"}
```

---

## Known differences

Two behaviours match Neo4j's *shape* but not its *result*. Both are listed here
rather than counted as equivalences.

**Full-text search is weaker.** `db.index.fulltext.queryNodes` runs PostgreSQL
full-text search with the `simple` dictionary — no stemming, no CJK
segmentation. Recall differs from Neo4j's Lucene index, most visibly for Korean,
and a hybrid ranking that fuses full-text with vector scores will weight the
full-text component differently as a result.

**Renaming a class renames the type.** `REMOVE n:Old SET n:New` is recognised
across the two clauses and applied to the catalog once, for every instance, in
constant time — not per node. The effect matches Neo4j; the cost and the
granularity do not. It applies only when `Old` is a real type and `New` is free;
anything else is a genuine retype and is refused.

---

## Differences from Neo4j worth knowing

**Labels are types, and types are hierarchical.** `MATCH (v:Vehicle)` returns
`Car` and `EV` instances without anyone attaching a second label. In Neo4j the
equivalent requires either multi-labelling every node or a `UNION`.

**Relationships have roles.** A relation type declares named participant slots
with type constraints, and the database enforces them:

```
ERROR:  role 'owner' of relation 'OWNS' requires a 'Person', got 'EV'
```

**Properties have declared types.** Undeclared properties still work — they land
in a `jsonb` extension column — but they do not get a typed column, an index, or
a statistics entry. The type system is optional, and you pay for what you skip.

**Errors carry corrections.** An unknown label reports the nearest valid names by
edit distance, which is what lets an agent fix itself in one retry:

```
ERROR:  unknown label 'Persn' in graph 'social'. did you mean: Person
```

**Results are `jsonb`, not an opaque type.** Any PostgreSQL driver reads them
without a special library, and any SQL query can join against them.

**Two ways in, one query path.** Cypher arrives over the PostgreSQL wire
protocol (spec 003, FR-024) or over **Bolt**, through the gateway beside the
database (spec 011, `bolt/`). A Neo4j driver connects to the gateway and the
application above it does not change — same sessions, same transactions, same
`Node` and `Relationship` objects. Both paths call the same `og_cypher()`, so
neither has semantics the other lacks: what fails here fails there, identically.

`tests/common/neo4j-movies/` runs the official Movie Graph sample — its dataset and all
24 guide queries — over **both** paths and compares every result against a Neo4j
instance holding the same data.
