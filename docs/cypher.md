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
| **vector** | `vector.similarity(a, b)`, `vector.distance(a, b)`, `vector.l2(a, b)` |

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

## Not supported in this release

| Construct | What happens | Alternative |
|---|---|---|
| `WITH` | parse succeeds, compile fails with an explanation | split the query, or wrap `og_cypher_sql()` output in a SQL CTE |
| `UNION` | parsed, not compiled | `UNION` the two `og_cypher()` calls in SQL |
| `CALL` / procedures | rejected at parse time | the `og_*` SQL function surface |
| `FOREACH` | rejected at parse time | `UNWIND` + write clause |
| multi-label patterns `(a:A:B)` | rejected with a suggestion | declare a common supertype — that is what the type system is for |
| `SET a:Label` / `REMOVE a:Label` | rejected | a node's type is part of its identity here; create a node of the target type |
| list comprehensions, pattern comprehensions | not parsed | `collect()` with a sub-query |
| `shortestPath` | not implemented | `og_vlp()` ordered by depth |

Check the current state programmatically:

```sql
SELECT og_cypher_check('MATCH (a) WITH a RETURN a');
-- {"ok": true, "clauses": 3, "write": false}   ← parses
SELECT og_explain_error('kb', 'MATCH (a) WITH a RETURN a');
-- {"ok": false, "code": "UNSUPPORTED_SYNTAX", "message": "WITH is not supported …"}
```

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
