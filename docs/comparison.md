# Comparison: Apache AGE and pgGraph

Three projects put graph traversal inside PostgreSQL. They are not three
implementations of one idea — they sit at different layers and give up different
things, and the choice between them is a choice about what you are willing to
lose.

> Sources for the pgGraph material are its own documentation, README and blog,
> retrieved 2026-08-16, against pgGraph 0.1.6 (alpha). Where a claim here is
> ours rather than theirs, it says so.

---

## What each of these actually is

| | Apache AGE | pgGraph | Ontological |
|---|---|---|---|
| kind | graph **store** | graph **index** | graph **store** |
| source of truth | AGE label tables | your existing relational tables | `og_data.*` |
| what it holds | nodes, edges, `agtype` properties | **topology only**, in a CSR array | nodes, edges, typed columns, adjacency segments |
| drop it and? | the data is gone | rebuild it | the data is gone |
| query surface | openCypher | fixed algorithm functions + a GQL 1.0 profile | Cypher and TypeQL |
| licence | Apache-2.0 | Apache-2.0 | Apache-2.0 |

pgGraph describes itself as "closer to a rebuildable graph index than a graph
database", and that is the honest framing. AGE and Ontological compete over how
a graph should be stored. pgGraph declines that question and accelerates
traversal over a schema someone else designed.

---

## Declaring the graph

### Apache AGE — label tables joined by inheritance

`create_graph('g')` creates a schema. Every label is a table that inherits from
one of two parents:

```sql
CREATE TABLE g._ag_label_vertex (id graphid PRIMARY KEY, properties agtype);
CREATE TABLE g._ag_label_edge   (id graphid PRIMARY KEY, start_id graphid,
                                 end_id graphid, properties agtype);

SELECT create_vlabel('g','Person');   -- g."Person" INHERITS (g._ag_label_vertex)
SELECT create_elabel('g','KNOWS');    -- g."KNOWS"  INHERITS (g._ag_label_edge)
```

The inheritance is machinery for scanning *all* vertices when a pattern carries
no label. It is not a type system: AGE has no `Car ISA Vehicle`, and a vertex
carries one label.

Properties live in one `agtype` column. There are no per-column statistics, no
`CHECK` constraints, and an index on a property has to be an expression index
written by hand. AGE also creates no index on `start_id`/`end_id`; our harness
creates all three before timing anything, because benchmarking it otherwise
would be a strawman ([`bench/harness.py`](../bench/harness.py)).

### pgGraph — register tables you already have

pgGraph owns no schema. You point it at existing tables and compile:

```sql
CREATE EXTENSION graph;
SELECT graph.add_table('public.customers'::regclass,
                       id_column := 'id', columns := ARRAY['name']);
SELECT graph.add_edge(from_table := 'public.orders'::regclass,
                      from_column := 'customer_id',
                      to_table := 'public.customers'::regclass, to_column := 'id',
                      label := 'placed_by', bidirectional := true);
SELECT * FROM graph.build();     -- compile forward + reverse CSR
```

`graph.build()` writes a `.pggraph` artifact; each PostgreSQL backend maps it
into backend-local anonymous memory as a private snapshot. Properties are not
copied — the engine returns source-table coordinates, or hydrates the original
rows on demand. Only columns registered as filterable participate in predicates.

There is no type hierarchy, no role constraint and no schema evolution. That is
a scope decision, not an omission.

### Ontological — one table per concrete type, real columns

```sql
SELECT og_create_type('kb','Vehicle','entity');
SELECT og_create_type('kb','EV','entity', ARRAY['Car']);
-- CREATE TABLE og_data.n_4 (id int8 PRIMARY KEY, __ext jsonb);
SELECT og_add_property('kb','EV','range_km','int4');
-- ALTER TABLE og_data.n_4 ADD COLUMN p_range_km int4;
```

Declared properties become columns ([`engine/src/catalog/types.rs`](../engine/src/catalog/types.rs));
undeclared ones fall into `__ext`. Inheritance is an interval label, so
`MATCH (v:Vehicle)` resolves to a fixed list of concrete tables at compile time
([`engine/src/cypher/views.rs`](../engine/src/cypher/views.rs)). Adjacency is
separate from the edge tables, packed 256 neighbours to a heap tuple
([`engine/src/storage/adjacency.rs`](../engine/src/storage/adjacency.rs)).

---

## Traversing

### AGE

```sql
SELECT * FROM cypher('g', $$ MATCH (a:Person)-[:KNOWS]->(b) RETURN b.name $$)
  AS (name agtype);
```

One hop is a B-tree probe on `KNOWS.start_id`, one random heap fetch per
matching edge row, another index lookup on `end_id`, then a JSON parse for the
property. Expanding a node of degree *d* costs *d* descents and *d* random
fetches.

A correction to how this repository has described AGE elsewhere: **it is not
true that the planner sees nothing.** AGE rewrites the `cypher()` call into a
real query tree during parse analysis. The measured gap comes from three
narrower places — `agtype` defeats column statistics and ordinary indexes,
adjacency is one row per edge, and the `*1..n` operator rescans. Our own
measurements say the third dominates: 799.50 ms at two hops, against 7.99 ms for
the same question written as fixed-length patterns
([`docs/benchmark.md`](benchmark.md)).

### pgGraph

```sql
SELECT graph.traverse('public.customers'::regclass, 42, max_depth := 6);
SELECT graph.shortest_path('public.a'::regclass, 1, 'public.b'::regclass, 99);
SELECT graph.search('name', 'acme', mode := 'contains');
SELECT graph.gql('...');       -- GQL 1.0 profile; openCypher is explicitly not supported
```

Finding neighbours is an array offset calculation. No index, no join, no
planner. Runaway expansion is held back by explicit depth limits, visited-set
tracking, frontier limits, pagination and OOM guards, because nothing else is
holding it back.

The cost of leaving the planner behind is that you get algorithms, not patterns.
There is no `MATCH (a)-[:X]->(b)<-[:Y]-(c) WHERE … RETURN count(*)`; you call
`traverse` or `shortest_path` and combine the results yourself.

### Ontological

Cypher compiles to ordinary SQL over the adjacency segments
([`engine/src/cypher/compile.rs`](../engine/src/cypher/compile.rs)):

```sql
SELECT n2.p_name
FROM og_data.v_2 n1
CROSS JOIN LATERAL (
  SELECT u.nbr, u.eid
    FROM og_data.og_adj adj1, LATERAL unnest(adj1.nbr, adj1.eid) AS u(nbr, eid)
   WHERE adj1.src = n1.id AND adj1.dir = 'o'::"char"
     AND adj1.etype = ANY(ARRAY[7]::int4[])
) u1
JOIN og_data.v_2 n2 ON n2.id = u1.nbr
WHERE n1.p_age > 30
```

One hop is one heap tuple. `n1.p_age > 30` is a column predicate the optimiser
can cost and index. Variable-length paths go through `og_vlp()`, a recursive SQL
function with trail semantics, LATERAL-joined per start row
([`engine/sql/access.sql`](../engine/sql/access.sql)).

---

## The axes that decide it

| | Apache AGE | pgGraph | Ontological |
|---|---|---|---|
| one hop | *d* index probes + *d* random fetches | array offset | one heap tuple |
| deep paths (10+) | times out at these scales, by its authors' account | designed for it | visited-set BFS in the heap, or a compiled CSR — [measured](deep-traversal.md) |
| arbitrary patterns | yes (openCypher) | no — fixed algorithms | yes (Cypher, TypeQL) |
| joins with your own SQL | result is `agtype` | results feed back into SQL | paste `og_cypher_sql()` into a CTE |
| type hierarchy | no | no | interval labels, constant-time |
| **row-level security mid-traversal** | applies (ordinary tables) | **does not** — topology lives outside the heap | applies |
| write visibility | transactional | trigger capture + **explicit apply**; can be stale | transactional |
| vectors | — | — | pgvector on nodes **and relationships** |
| start-up cost | none | per-backend engine load | none |

The two bold rows are where the architectural bet actually shows. pgGraph's
graph is a structure outside PostgreSQL's heap, so a path through a row the
caller may not read can still appear in a result, and an edge committed a moment
ago is invisible until the sync is applied. Neither can happen here — and we pay
MVCC and heap cost on every hop for that.

---

## Numbers

### Against AGE

Ours, on identical data, with every answer checked for equality before any
timing is reported — full method in [`docs/benchmark.md`](benchmark.md).
50,000 nodes / 974,936 edges, median latency:

| query | Ontological (Cypher) | Apache AGE | AGE without `*1..n` |
|---|---|---|---|
| 1 hop | 2.10 ms | 2.15 ms | 2.15 ms |
| 2 hops | 3.96 ms | 799.50 ms | 7.99 ms |
| 3 hops | 33.86 ms | 22,412 ms | 34.63 ms |

Written as fixed-length patterns, AGE is at parity with us. The collapse is one
operator, not the storage.

### pgGraph's published numbers

pgGraph publishes no comparison against AGE at all — its own post says so:
*"We do not plot Apache AGE here, as deep traversals (10+ hops) on datasets of
this size resulted in query timeouts under the recursive SQL model."* What
follows is therefore a single-system measurement, reproduced as published.

**Read the tables carefully.** In the rendered page each row is
`[label] [bar value] [Cold] [Hot]`, and converting it naively swaps the two
columns. Cold is the slow one:

PANAMA — 2,016,523 nodes / 5,802,586 edges:

| query | Cold (ms) | Hot (ms) |
|---|---|---|
| Status | 900.2 | 32.2 |
| Entity Search | 1005.6 | 353.9 |
| Traverse Depth 2 | 699.6 | 117.3 |
| Shortest Path | 491.5 | 4.0 |
| Component Stats | 651.2 | 157.3 |
| Largest Component | 1124.4 | 613.2 |

LDBC SNB — 3,181,724 nodes / 34,512,076 edges:

| query | Cold (ms) | Hot (ms) |
|---|---|---|
| Status | 2870.4 | 27.2 |
| Person Search | 2762.0 | 9.8 |
| Friend Traversal Depth 1 | 2806.0 | 34.1 |
| Person Content Neighborhood | 3008.8 | 177.8 |
| Forum Neighborhood | 3014.1 | 181.7 |
| Post To Tag Path | 2825.5 | 6.5 |
| Tag To Tagclass Path | 2979.0 | 7.0 |
| Component Stats | 3425.9 | 428.4 |

Cold is defined as a Docker restart before each query, excluding
`graph.build()`; hot is one persistent psycopg backend after an unrecorded
warm-up.

Four observations, none of which require the numbers to be wrong:

- **The cold column is the architecture, not the query.** On LDBC every cold
  measurement lands between 2.76 s and 3.43 s regardless of what was asked. That
  is the cost of mapping a 34.5 M-edge CSR artifact into a backend, and each new
  backend pays it. Connection-pooled services amortise it; serverless and
  per-request-connection deployments do not.
- **The deepest traversal shown is depth 2.** The surrounding text argues for
  microsecond latency at 10 and 20 hops; the tables contain a *Traverse Depth 2*
  and a *Friend Traversal Depth 1*, in milliseconds. The claim we would most want
  to check is the one not measured.
- **No correctness gate is described.** Ours voids its own timings when two
  systems disagree on an answer; that is the property that makes a comparison
  mean anything, and it is the reason these two sets of numbers cannot be put in
  one table.
- **Hardware, result-set sizes and configuration are not disclosed**, so nothing
  above is comparable with the AGE table. For scale only, and not as evidence of
  anything: our one hop is 2.10 ms on 50 K nodes / 975 K edges, their depth-1
  friend traversal is 34.1 ms on 3.2 M / 34.5 M. Different data, different
  machines, different result cardinality.

---

## Where pgGraph's design beats ours

Stated plainly, because the rest of this document is easier to write.

**Deep traversal.** `og_vlp()` is a recursive SQL function. Trail semantics stop
cycles from expanding forever and it does not rescan the edge table the way
AGE's operator does — but it is still in the recursive-CTE family, the frontier
still materialises in a worktable, and every step still pays heap and MVCC cost.
We measure 33.86 ms at three hops and are 11× behind Neo4j there.

Ten and twenty hops used to be a range this document said we had not measured
and had no argument for. They have since been measured, and the answer split
into two claims of very different size ([`deep-traversal.md`](deep-traversal.md)).
On 50,000 nodes / 999,784 edges, counting the distinct nodes within six hops:

| | median |
|---|---|
| `og_vlp` — enumerate trails, as before | 49,334 ms |
| `og_reach` — visited-set BFS, still in the heap, MVCC and RLS intact | 71 ms |
| `og_csr_reach` — compiled backend-local CSR, pgGraph's shape | 4.9 ms |

Most of what looked like an architectural gap was ours: Cypher's variable-length
match yields one row per *path*, so `og_vlp` was producing `degreeᵏ` rows to
answer a question bounded by `|V|`. Asking it as reachability is 691× faster
without giving up a single guarantee, and the Cypher compiler now picks that
path automatically when no path variable is bound and the projection cannot
observe multiplicity.

What remains is genuinely pgGraph's, and it is about 15×: the pointer-free hot
loop over a contiguous array. The interesting part is how close the two designs
already are — `og_adj` *is* a CSR layout, and the only difference is which side
of the heap it sits on. `og_csr_build()` now builds the outside version too, and
its costs are pgGraph's costs exactly: a per-backend compile (119 ms, 8.4 MiB
here), a snapshot frozen until rebuilt, and no RLS. Which is why Cypher does not
route to it; it is exposed, measured, and left as an explicit choice.

---

## Choosing

- Your data is already relational, you want bounded deep traversal over foreign
  keys, and you can accept a derived index that is rebuilt, that is invisible to
  RLS and that may lag a commit → **pgGraph**.
- You are moving a property-graph application onto PostgreSQL and want
  openCypher compatibility more than traversal depth → **Apache AGE**.
- You want the graph to *be* PostgreSQL data — typed columns, an ontology with
  inheritance and roles, embeddings on relationships, row-level security holding
  mid-traversal, and Cypher that compiles into SQL you can join against →
  **this project**, with three-hop latency that beats AGE's `*1..n` and loses to
  Neo4j.
