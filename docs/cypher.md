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

A label matches the type **and all of its subtypes**. That is resolved once at
compile time through the interval index, so it costs nothing per row.

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
arithmetic    +  -  *  /  %  ^  ||
conditional   CASE … WHEN … THEN … ELSE … END
```

### Functions

| Group | Functions |
|---|---|
| aggregate | `count` (incl. `DISTINCT` and `count(*)`), `sum`, `avg`, `min`, `max`, `collect`, `stdev` |
| graph | `id`, `labels`, `type`, `nodes`, `relationships`, `length`, `size` |
| string | `toUpper`, `toLower`, `trim`, `substring`, `replace`, `split` |
| numeric | `abs`, `ceil`, `floor`, `round`, `sqrt`, `rand` |
| conversion | `toString`, `toInteger`, `toFloat` |
| temporal | `timestamp`, `datetime` |
| other | `coalesce`, `keys`, `exists` |
| **vector** | `vector.similarity(a, b)`, `vector.distance(a, b)`, `vector.l2(a, b)`, `genai.vector.encode(text, provider, config)` |

`vector.similarity` compiles to `1 - (a <=> b)` on a pgvector column, so it uses
the HNSW index when the query is shaped for it.

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
| multi-label patterns `(a:A:B)` | the most specific label decides; see "Differences" |
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
| `UNION` | parsed, not compiled | `UNION` the two `og_cypher()` calls in SQL |
| `FOREACH` | rejected at parse time | `UNWIND` + write clause |
| `SET a:Label` adding a *new* label | rejected with the reason | a node's type is part of its identifier here. To rename a class write `REMOVE n:Old SET n:New`; to move a node between types, create it under the target type |
| pattern comprehensions | not parsed | `collect()` with a sub-query |
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

`tests/neo4j-movies/` runs the official Movie Graph sample — its dataset and all
24 guide queries — over **both** paths and compares every result against a Neo4j
instance holding the same data.
