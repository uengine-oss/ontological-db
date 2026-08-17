# Benchmark: Neo4j vs Apache AGE vs TypeDB

Three graph databases with three different answers to the same question — how
should a graph be stored and traversed — measured on one machine, one dataset
and one set of queries, with the answers checked before any timing is reported.

Ontological (this project) and a hand-written recursive CTE are in every table
as well: the first because this is its repository, the second because a graph
benchmark without a plain-SQL floor is not a benchmark, it is an advertisement.

> Numbers, method and the harness that produced them are in this repository.
> `bench/harness.py` is the whole apparatus; `bench/results/*.json` are the raw
> runs. Everything below can be reproduced with the commands in
> [Reproducing](#reproducing).

---

## The systems under test

| | model | query language | storage of adjacency | properties |
|---|---|---|---|---|
| **Neo4j 5.26** | native property graph | Cypher | doubly-linked relationship records, first-relationship pointer per node | property store, dynamic records |
| **Apache AGE 1.5** | property graph inside PostgreSQL | Cypher via `cypher()` function | one heap row per edge; endpoint B-trees only if you create them yourself | `agtype` (JSON) column |
| **TypeDB 3.12** | typed entity–relation–attribute (ERA) | TypeQL | relations are first-class objects with roles | attributes are objects, owned by entities |
| Ontological | typed property graph inside PostgreSQL | Cypher via `og_cypher()` | CSR-style segments, ≤256 neighbours per heap tuple | real typed columns |
| recursive CTE | two tables | SQL | B-tree on `(src)` and `(dst)` | ordinary columns |

The three headline systems differ in kind, not degree:

- **Neo4j** is the incumbent native engine. A relationship record holds pointers
  to the next relationship of both endpoints, so a hop is a pointer chase, not
  an index lookup. It is a separate JVM server reached over bolt.
- **Apache AGE** puts Cypher on PostgreSQL by handing the query to the server as
  *a string inside a function call*. Edges are ordinary rows and properties are
  JSON. Every hop costs `degree` index probes plus `degree` random heap fetches,
  and the planner cannot see inside the pattern.
- **TypeDB** is not a property graph at all. Relations are typed objects that
  play roles, attributes are deduplicated value objects, and the schema is
  checked on write. Its query language has no variable-length path operator, so
  a `*1..3` traversal has to be written as an explicit disjunction of depths —
  a modelling difference that shows up directly in the numbers.

---

## Method

The harness is written to be hostile to its own conclusions.

**One generator, one graph.** A seeded generator produces the edge list; each
system loads that same list through its own bulk path. No system sees data the
others did not.

**Answers are checked before timings are reported.** Every system runs the same
logical queries and the results are compared. If they disagree, the harness
prints the disagreement and marks those timings void. This is not ceremony — an
earlier version of the harness caught the systems starting from *different*
nodes because the start property was not unique, and every number before that
fix was meaningless. In the runs below, every system returned identical answers
for every query.

**Timing is per-statement on an already-open connection.** Spawning a client per
query costs ~12 ms, which is more than most of the queries being compared;
measuring that way makes every system look identical. PostgreSQL systems are
timed with psql's `\timing`; Neo4j and TypeDB are timed client-side on a reused
session with the result fully consumed. Both measure the same thing: the time
from issuing a statement to holding its answer.

**The protocol floor is published next to the latency.** A trivial query is not
free, and it is not equally expensive on every client path. Reading a one-hop
latency without the floor next to it will mislead you about Neo4j in particular.

**Every query text is warmed, and the clients are warmed too.** Each query runs
from five different start nodes, and warm-up covers all five, so no system is
charged for planning a text another system had already planned. The Python
drivers additionally need *tens* of calls to reach steady state — on an empty
query, bolt reports 2.05 ms after 2 warm-up calls and 0.73 ms after 50 — so
those clients get 50 warm-up calls. psql was checked for the same effect and
does not have one (first ten calls 0.710 ms, last ten 0.679 ms over 40), which
is why the warm-up is asymmetric: it is compensating for a measured client
artefact, not handing anyone an advantage. An earlier draft of this document
had Neo4j at 1.8 ms for every query; that number was the driver warming up.

**Each system gets the indexes a competent operator would create.** This turned
out to matter more than anything else in the comparison. AGE creates no index
beyond a primary key on `id` — not on edge endpoints, not on properties — so
benchmarking it as-installed measures a misconfiguration, not a database. With
the three indexes its documentation calls for, AGE's one-hop time at 50k drops
from 29.6 ms to 1.6 ms and its property scan from 12.1 ms to 0.27 ms.
Ontological was found to have the same gap and was given `og_create_index` on
the same property for the same reason. Neo4j has a range index on `:P(val)`;
TypeDB has `@key`; the CTE has B-trees on `n(val)`, `e(src)`, `e(dst)`.

**Page counts are reported for the PostgreSQL systems.** Latency moves with
cache state; logical page accesses are a direct function of the storage layout.
Neo4j and TypeDB have no comparable counter exposed, so those cells are empty
rather than filled with something that looks like the same measurement.

### The workload

| query | what it asks |
|---|---|
| `1hop` | out-neighbours of one node, counted |
| `2hop` | distinct nodes within 2 hops of one node |
| `3hop` | distinct nodes within 3 hops of one node |
| `prop_scan` | count of nodes whose `val < 100` |

Written out in the three languages, the same 2-hop question is:

```cypher
-- Neo4j, Apache AGE, Ontological
MATCH (a:P {val: 7})-[:K*1..2]->(b:P) RETURN count(DISTINCT b)
```

```typeql
# TypeDB — no *1..n operator, so the depths are spelled out
match $a isa P, has val 7;
  { $r1_0 isa K (src: $a, dst: $b); } or
  { $r2_0 isa K (src: $a, dst: $m2_0); $r2_1 isa K (src: $m2_0, dst: $b); };
select $b; distinct; reduce $c = count;
```

Because TypeQL forces the depths to be written out, AGE is also measured a
second way — `age_explicit`, the same question asked as a union of fixed-length
patterns instead of `*1..n`. Without it the numbers would support the claim
"AGE's storage is slow", which turns out not to be what they show.

---

## Environment

- Apple silicon (arm64), macOS, Docker Desktop; all servers on the same host,
  all clients on the host, all connections over published ports.
- PostgreSQL 16.14 (Debian) — Ontological, Apache AGE 1.5.0, recursive CTE.
- Neo4j 5.26.28 Community, 2 GB heap / 1 GB page cache.
- TypeDB 3.12.1, default configuration.
- Clients: psql 18.4, `neo4j` Python driver 6.2.0, `typedb-driver` 3.12.1.

Community/default configurations throughout. No engine was tuned for this
workload, which cuts both ways: none of them is showing its best possible
number, and none was handicapped.

---

## Results

Median latency. Every system returned identical answers for every query at both
scales (1 hop = 8, 2 hops = 75 at the small scale; 15 and 359 at the large one),
so nothing here is void.

### 5,000 nodes / 37,588 edges (average degree 8)

| query | Neo4j 5 | Apache AGE | AGE, explicit depths | TypeDB 3 | Ontological | Ontological (raw) | recursive CTE |
|---|---|---|---|---|---|---|---|
| 1 hop | 0.77 ms | 0.45 ms | 0.46 ms | **0.37 ms** | 0.78 ms | 0.24 ms | 0.23 ms |
| 2 hops | 0.80 ms | 26.86 ms | 1.53 ms | **0.62 ms** | 1.11 ms | 0.35 ms | 0.31 ms |
| 3 hops | **0.81 ms** | 157.24 ms | 6.74 ms | 2.00 ms | 3.62 ms | 0.89 ms | 0.48 ms |
| property scan | 0.76 ms | 0.32 ms | 0.24 ms | 0.39 ms | 0.64 ms | 0.20 ms | 0.20 ms |
| *protocol floor* | *0.71 ms* | *0.21 ms* | *0.14 ms* | *0.35 ms* | *0.20 ms* | *0.16 ms* | *0.23 ms* |

### 50,000 nodes / 974,936 edges (average degree 20)

| query | Neo4j 5 | Apache AGE | AGE, explicit depths | TypeDB 3 | Ontological | Ontological (raw) | recursive CTE |
|---|---|---|---|---|---|---|---|
| 1 hop | 0.74 ms | 2.15 ms | 2.15 ms | **0.46 ms** | 2.10 ms | 0.31 ms | 0.24 ms |
| 2 hops | **0.92 ms** | 799.50 ms | 7.99 ms | 1.48 ms | 3.96 ms | 0.50 ms | 0.36 ms |
| 3 hops | **2.99 ms** | 22,412 ms | 34.63 ms | 19.73 ms | 33.86 ms | 4.33 ms | 3.49 ms |
| property scan | 0.68 ms | 0.27 ms | 0.24 ms | 0.52 ms | 0.62 ms | 0.24 ms | 0.19 ms |
| *protocol floor* | *0.79 ms* | *0.17 ms* | *0.18 ms* | *0.37 ms* | *0.19 ms* | *0.19 ms* | *0.18 ms* |

Read the floor row before drawing conclusions about Neo4j: at 1 and 2 hops its
measured latency is at or barely above what an *empty* query costs through the
same client. Its engine time there is below what this method can resolve. Three
hops is the only column where Neo4j is doing work this benchmark can see.

### Logical page accesses (PostgreSQL-resident systems only)

| query | Apache AGE | AGE, explicit depths | Ontological | Ontological (raw) | recursive CTE |
|---|---|---|---|---|---|
| 1 hop | 1,707 | 1,707 | 1,742 | 389 | 8 |
| 2 hops | 48,508 | 15,481 | 3,340 | 1,335 | 420 |
| 3 hops | 48,523 | 97,721 | 32,004 | 6,510 | 8,898 |
| property scan | 35 | 35 | 1,174 | 8 | 6 |

The AGE edge table at this scale is about 7,200 pages (measured at 135 edge rows
per page). Its `*1..n` reads 48,523 — the whole table around seven times over —
and reads *the same* 48,523 whether the question is two hops or three. The page
count has stopped depending on the question, which is the signature of a scan
rather than a traversal. Written as explicit depths, the same two-hop question
reads 15,481.

---

## What the numbers say about each system

### Apache AGE: the storage is fine, the `*1..n` operator is not

This is the finding that reorganised the whole document. AGE's headline numbers
look catastrophic — 22.4 seconds for a 3-hop question on a million-edge graph,
7,500× Neo4j — and the obvious conclusion is that storing edges as heap rows
with JSON properties cannot compete. That conclusion is wrong.

Asked the *same question* without the variable-length operator — as a union of
fixed-length patterns, which is exactly what TypeQL forces TypeDB to do — AGE
answers in **34.63 ms instead of 22,412 ms**, a factor of 647. At two hops it is
7.99 ms instead of 799.50 ms, a factor of 100. Same data, same indexes, same
engine, same answers.

`EXPLAIN` shows why. This is the two-hop query at 50k, with every index in
place. The expansion runs inside a function scan the planner cannot see into,
and then the terminal node has to be rejoined by brute force:

```
->  Nested Loop  (actual time=633..2731 rows=360)
      Join Filter: age_match_vle_terminal_edge(a.id, b.id, _age_default_alias_0.edges)
      Rows Removed by Join Filter: 17999640          <-- 18M rows built, 360 kept
      ->  Seq Scan on "P" b  (rows=50000)            <-- the entire vertex table
      ->  Materialize
            ->  Function Scan on age_vle             <-- 627 ms, opaque to the planner
```

So the honest statement about AGE is narrower and more useful than the headline:
**its storage layout is competitive, its planner integration at the `cypher()`
boundary is not.** One-hop traversal (2.15 ms) and property scan (0.27 ms, the
fastest of the three products) are perfectly respectable. If you use AGE, avoid
`*1..n` and write the depths out; you will get two to three orders of magnitude
back.

One caveat in AGE's favour and one against:

- **In its favour:** as installed, AGE creates no index on edge endpoints or
  properties. With the indexes its documentation calls for, the one-hop time at
  50k fell 18× (29.6 → 1.6 ms) and the property scan 45× (12.1 → 0.27 ms). Any
  AGE benchmark that skips this step is measuring a misconfiguration.
- **Against:** the `*1..n` times are not just slow, they are wildly variable —
  median 22.4 s at 3 hops, p95 **914 s**. One unlucky start node cost fifteen
  minutes.

### Neo4j: flat, and mostly too fast to measure here

Neo4j is the only system whose latency barely responds to the workload: 0.74 /
0.92 / 2.99 / 0.68 ms across the four queries at a million edges. Going from
37,588 to 974,936 edges — 26× the data, and roughly 16× the nodes reachable in
three hops — moved its 3-hop time from 0.81 ms to 2.99 ms, a factor of 3.7. It
scales *better* than the frontier grows, which is what a linked-record store
with a first-relationship pointer per node is supposed to do.

At 1 and 2 hops the honest reading is that this benchmark cannot see Neo4j's
engine at all: 0.74 ms against a 0.79 ms empty-query floor. Anything faster than
its own protocol is invisible.

The cost is on the other side of the ledger: it is a separate JVM server with
its own storage, its own operational surface and its own bolt protocol, and the
Python client's warm-up ramp (2.05 ms → 0.73 ms over fifty calls) is a real cost
that short-lived processes pay.

### TypeDB: fastest lookups, no path operator, and a write throughput problem

TypeDB has the fastest one-hop time of the three at both scales — 0.46 ms at
50k, against 0.74 ms for Neo4j and 2.15 ms for AGE — and that time barely moved
across the two scales (0.37 → 0.46 ms for 26× the data). A `@key` lookup goes
straight to the attribute object; the typed schema is doing real work here, not
just documentation. On the property scan it is second (0.52 ms) behind AGE
(0.27 ms).

Traversal is a different story, and the reason is in the language rather than
the engine. **TypeQL has no `*1..n`**, so a bounded-depth traversal has to be
written as a disjunction that re-walks its own prefixes:

```typeql
{ K(src:$a, dst:$b); } or
{ K(src:$a, dst:$m); K(src:$m, dst:$b); } or
{ K(src:$a, dst:$m); K(src:$m, dst:$n); K(src:$n, dst:$b); };
```

At three hops that costs 19.73 ms — 6.6× Neo4j, though still 1,100× better than
AGE's `*1..n`. Its 3-hop time grew 9.9× between the two scales, against Neo4j's
3.7×. Part of that is the redundant prefix walking rather than the storage
engine, and a recursive TypeQL function might do better; that variant was not
tested, so treat the traversal numbers as "TypeDB as most people would write
this query", not "the best TypeDB can do".

The operational findings are harder to work around:

- **Writes are two orders of magnitude slower than everyone else.** 4,186
  relations/s at the large scale, against 103,918/s for Neo4j and 429,790/s for
  AGE. Loading the million-edge graph took **3 minutes 53 seconds** versus 9.4
  seconds for Neo4j. That is with 8 concurrent write transactions; a single
  connection does ~600/s.
- **Batching makes it worse, not better.** Putting several relations in one
  `match … insert …` query slows the load down — 500 edges/s at one pair per
  query, 240 at five — because the pattern gets quadratically harder to plan.
  The fast path is the one that looks wasteful.
- **A large write transaction fails.** The first attempt at the million-edge
  load used 8 transactions of ~122,000 relations each and died with `[TSV13]
  Execution interrupted by a concurrent transaction close` after 30 minutes. The
  loader in this repository chunks at 5,000 relations per transaction with a
  retry, which works.
- **The server was OOM-killed** (exit 137) shortly after the large run, on a
  15.6 GB Docker VM with no explicit memory limit set.

### The two reference points

**Ontological** (this repository) sits between the incumbents: better than AGE's
`*1..n` by 660× at three hops, worse than Neo4j by 11×. The gap between its
Cypher surface (33.86 ms) and its own storage path (4.33 ms) is 7.8× — that is
the query engine's overhead, and it is the clearest optimisation target in the
codebase. Its property scan is also the weakest of the PostgreSQL systems
(1,174 pages against AGE's 35), because the scan still walks the type table.

**The recursive CTE** wins nearly every cell. Two indexed tables and no query
engine is very hard to beat on a workload this shape, and any product in this
table has to justify its existence against that, not against the other products.
Where it stops being the answer is not in this benchmark: no schema, no
inheritance, no Cypher, and the query has to be hand-written for every depth.

---

## Summary

| | Neo4j 5 | Apache AGE | TypeDB 3 | Ontological |
|---|---|---|---|---|
| 1 hop @ 1M edges | 0.74 ms *(at protocol floor)* | 2.15 ms | **0.46 ms** | 2.10 ms |
| 3 hops @ 1M edges | **2.99 ms** | 22,412 ms — or 34.63 ms without `*1..n` | 19.73 ms | 33.86 ms |
| property lookup | 0.68 ms | **0.27 ms** | 0.52 ms | 0.62 ms |
| bulk load | 103,918 edges/s | **429,790 edges/s** | 4,186 edges/s | 124,580 edges/s |
| scaling, 3 hops, 26× data | **3.7×** | 143× (`*1..n`) / 5.1× (explicit) | 9.9× | 9.4× |
| variable-length paths | native, fast | native, unusable | **not in the language** | native |
| deployment | separate JVM server | PostgreSQL extension | separate server | PostgreSQL extension |

Load rates are each system's own practical bulk path, not the same mechanism:
the PostgreSQL systems load through SQL `INSERT … SELECT`, Neo4j through batched
`UNWIND` over bolt, TypeDB through its driver one relation at a time. They say
what it costs to get a graph in, not how fast each engine writes.

---

## What this benchmark does not show

- **One machine, one process, no concurrency.** Every query ran alone. Nothing
  here says anything about throughput under load, locking, or multi-user
  behaviour — which is where a JVM server and a PostgreSQL extension differ most.
- **Two scales, one graph shape.** A uniform random graph with fixed average
  degree. No hubs, no communities, no skew — and skew is exactly what breaks
  traversal planners.
- **Four queries.** No writes under measurement, no pattern matching with
  predicates, no aggregation, no shortest path, no LDBC SNB.
- **Nothing deeper than three hops.** Four and beyond is where the shape of the
  traversal starts to matter more than the storage, and it has its own document
  and its own harness: [`deep-traversal.md`](deep-traversal.md),
  [`bench/csr/`](../bench/csr/). The `og_vlp` numbers here are the path a query
  still takes when it binds a path variable; a query that cannot observe path
  multiplicity is now compiled to a visited-set BFS instead.
- **Community and default configurations.** Neo4j Enterprise, a tuned TypeDB, or
  a PostgreSQL with a larger `shared_buffers` would all move.
- **Warm cache throughout.** Every number is steady-state on a warm cache; cold
  start is not measured.
- **pgGraph is not in *this* table.** Its published figures (PANAMA and LDBC
  SNB, cold and hot) come from a different dataset, a different machine and an
  undisclosed configuration, with no correctness gate described, so quoting them
  next to these would be exactly the advertisement this document is trying not
  to be. That objection does not apply to a build we compile and run ourselves:
  pgGraph 1.1.0 is built from source and measured against Neo4j, AGE and this
  project on one machine and one dataset in
  [`deep-traversal.md`](deep-traversal.md). What it is, and where its design
  beats ours: [`comparison.md`](comparison.md).
- **Some systems were measured in separate runs** against the same generated
  graph, because the TypeDB load takes four minutes and its server had to be
  restarted after an OOM kill. Same machine, same seed, same edge list, same
  session-level method — but not the same wall-clock minute.

The results files record which run each number came from.

---

## Reproducing

```bash
docker run -d --name bench-neo4j  -p 27687:7687 \
  -e NEO4J_AUTH=neo4j/benchpass123 \
  -e NEO4J_server_memory_heap_max__size=2G \
  -e NEO4J_server_memory_pagecache_size=1G neo4j:5
docker run -d --name bench-typedb -p 21729:1729 typedb/typedb:latest
pip install neo4j typedb-driver

python3 bench/harness.py --scale 5000  --degree 8  --runs 10
python3 bench/harness.py --scale 50000 --degree 20 --runs 8

# the two AGE variants side by side, on one loaded database
python3 bench/harness.py --scale 50000 --degree 20 --systems age,age_explicit
```

The numbers in this document come from these runs:

| | 5,000 nodes | 50,000 nodes |
|---|---|---|
| Ontological, raw, CTE | `bench-5000-20260806T042903Z` | `bench-50000-20260806T042833Z` |
| AGE, AGE explicit | `bench-5000-20260806T043920Z` | `bench-50000-20260806T052220Z` |
| Neo4j, TypeDB | `bench-5000-20260806T043214Z` | `bench-50000-20260806T043634Z` |

Each file records the engine versions, load throughput, protocol floor,
correctness answers, p95s and page counts quoted above.

The Studio renders the same files as a report page at
[http://localhost:7474/benchmark.html](http://localhost:7474/benchmark.html)
— it reads `bench/results/` through `GET /api/benchmark` and picks the newest
run per system, so re-running the harness updates the page with no edit
anywhere.
