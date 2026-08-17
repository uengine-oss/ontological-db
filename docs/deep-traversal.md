# Deep traversal: what the recursive CTE actually costs, and what pgGraph's bet buys

[`comparison.md`](comparison.md) ends by conceding deep traversal to pgGraph and
saying that a cached in-memory mirror "is the obvious thing to consider, and it
is not built." This document is what happened when it was built and measured.

The short version, and it is not the one the concession anticipated:

- **Most of the cost was not the heap.** `og_vlp()` enumerates *trails* — every
  distinct edge-sequence — so its row count is `degreeᵏ`. On a graph of average
  degree 20 that is 20× per extra hop, and at six hops it is 49 seconds. Asking
  the same question as reachability, with a visited set, is bounded by `|V|+|E|`
  and answers in **71 ms in the heap** with MVCC and RLS fully intact. That is a
  **691× improvement with nothing lifted out of PostgreSQL.**
- **Leaving the heap is worth another 15×, not another 1000×.** The compiled CSR
  answers the same question in 4.86 ms. Real, and the pgGraph architecture earns
  it — but it is the second-largest factor here, not the first.
- **The Cypher surface now picks between them by cost**, so
  `MATCH (a)-[:K*1..8]->(b) RETURN count(DISTINCT b)` went from *not finishing*
  to 277 ms without any change to what Cypher means.

Everything below is reproducible with [`bench/csr/`](../bench/csr/).

---

## The mistake, stated precisely

Cypher's variable-length match yields **one row per path**. `og_vlp` implements
that faithfully: it carries an `int8[]` of edge ids, rejects an edge that
repeats (trail semantics), and returns a row per walk. So `RETURN count(b)`
counts walks and `RETURN count(DISTINCT b)` counts nodes — two different
questions, and the second is the one applications ask.

Answering the second by enumerating the first costs `Σ degreeⁱ` rows for an
answer bounded by `|V|`. At depth 6 on the dense fixture that is roughly 67
million walks to report 50,000 nodes.

This is not the pathology `docs/benchmark.md` documents in Apache AGE, which is
a rescan — `og_vlp` genuinely walks the graph. It is a different mistake with a
similar shape: doing work proportional to the question's *form* rather than its
*answer*.

---

## Four ways to ask it

| variant | what it is | keeps |
|---|---|---|
| `og_vlp` | today's recursive CTE, trails + path array | everything |
| `og_reach_sql` | same CTE, no path array, `UNION` not `UNION ALL` | everything |
| `og_reach` | Rust BFS, visited set, adjacency read through SPI | everything |
| `og_csr_reach` | backend-local compiled CSR, no SPI, no heap, no planner | nothing but the answer |

`og_reach_sql` is the interesting cheap one: dropping the path column lets
PostgreSQL's `UNION` deduplicate the worktable against everything produced so
far. That is not a visited set — a node found at depth 2 is produced again at
depth 3 because `(node, depth)` is a different row — so it is `O(k·|V|)` rather
than `O(|V|+|E|)`. It needed no new code at all.

`og_reach` keeps MVCC, row-level security and this transaction's uncommitted
writes, because it reads the same heap tuples the rest of the engine does.
`og_csr_reach` compiles the topology into backend-local `u32` arrays once and
walks them with nothing in the loop; it sees a frozen snapshot and never
consults RLS. That difference is the whole architectural question, and it is now
measured rather than argued.

---

## Method

Same discipline as [`docs/benchmark.md`](benchmark.md), which is where the
details live. Briefly: five fixed start nodes spread across the id space, one
warm-up before timing, medians reported, per-statement timing inside a single
psql session, and — the part that matters — **every variant's answers are
compared before any timing is reported.** `bench/csr/deep.py` records the
answers in its JSON output and exits non-zero on disagreement. In every run
below all four variants returned identical answers at every depth.

The CSR compile is timed separately and reported on its own, because it is a
per-backend cost the query does not pay.

Two graph shapes, uniform random, fixed seed:

- **dense** — 50,000 nodes / 999,784 edges, average degree 20. Saturates at
  depth 4, which is where trail enumeration explodes.
- **sparse** — 200,000 nodes / 799,988 edges, average degree 4. Saturates around
  depth 9, so depth actually means something out to 20.

PostgreSQL 16.14 in Docker on Apple silicon, community configuration, warm
cache. One machine, one process, no concurrency.

---

## Results

Median latency in milliseconds for *"count the distinct nodes within k hops"*.
`—` means the variant was not run at that depth because the previous one had
already established it was hopeless.

### dense — 50,000 nodes / 999,784 edges (degree 20)

| depth | `og_vlp` | `og_reach_sql` | `og_reach` | `og_csr_reach` |
|---|---|---|---|---|
| 1 | 0.17 | 0.15 | 0.08 | **0.05** |
| 2 | 0.51 | 0.34 | 0.20 | **0.09** |
| 3 | 6.85 | 4.07 | 1.61 | **0.60** |
| 4 | 106.72 | 49.24 | 23.62 | **3.68** |
| 5 | 2,300.22 | 257.44 | 58.97 | **4.81** |
| 6 | 49,333.99 | 426.45 | 71.42 | **4.86** |
| 7 | — | 760.29 | 74.44 | **5.50** |
| 8 | — | 910.60 | 74.93 | **5.92** |
| 10 | — | 1,493.60 | 72.71 | **4.84** |
| 16 | — | 2,939.31 | 77.25 | **5.61** |
| 20 | — | 3,659.57 | 69.43 | **4.88** |

CSR compile: 8.4 MiB, 119 ms, once per backend.

### sparse — 200,000 nodes / 799,988 edges (degree 4)

| depth | `og_vlp` | `og_reach_sql` | `og_reach` | `og_csr_reach` |
|---|---|---|---|---|
| 1 | 0.17 | 0.14 | 0.08 | **0.06** |
| 2 | 0.19 | 0.15 | 0.12 | **0.06** |
| 3 | 0.35 | 0.25 | 0.25 | **0.07** |
| 4 | 0.94 | 0.66 | 0.58 | **0.10** |
| 5 | 2.98 | 1.49 | 1.11 | **0.18** |
| 6 | 7.21 | 4.55 | 2.96 | **0.49** |
| 8 | — | 64.04 | 44.33 | **4.96** |
| 10 | — | 778.75 | 150.67 | **19.73** |
| 12 | — | 1,930.32 | 205.42 | **23.07** |
| 16 | — | 4,674.51 | 206.09 | **21.14** |
| 20 | — | 7,294.30 | 205.81 | **21.13** |

CSR compile: 9.2 MiB, 229 ms, once per backend.

### Reading these

**`og_vlp`'s curve is the degree, exactly.** 0.51 → 6.85 → 106.72 → 2,300 →
49,334 is a factor of 13, 16, 22, 21 per hop on a graph of average degree 20.
It is not slowing down because of the heap, MVCC or the planner; it is producing
`degree` times as many rows each hop, as designed. Nothing about where the
topology is stored would change that.

**Both reachability variants go flat once the graph is covered.** On the dense
fixture `og_reach` is 71 ms at depth 6 and 69 ms at depth 20 — after the whole
graph is reachable there is nothing left to do, so latency stops depending on
depth. That flatness, not the constant, is what "20+ hops" actually requires.
`og_reach_sql` never gets it, because re-emitting each node once per level keeps
`O(k·|V|)` work on the clock: 426 ms at 6, 3,660 ms at 20.

**The CSR's win is a single order of magnitude, and a stable one.** Dense depth
6: 71.42 → 4.86 ms, a factor of 14.7. Dense depth 20: 69.43 → 4.88, 14.2.
Sparse depth 20: 205.81 → 21.13, 9.7. That is the price of the heap — tuple
deforming, MVCC visibility, SPI setup per level — and it is a real number worth
having, but it is one order of magnitude against the three that changing the
algorithm bought.

**The CSR's floor is row output, not traversal.** Sparse jumps from 4.96 ms at
depth 8 to 19.73 ms at depth 10 with no change in the traversal — that is where
the reachable set saturates at ~200,000 nodes and the cost becomes the cost of
returning 200,000 rows. Below saturation it is sub-millisecond.

---

## End-to-end, through Cypher

The functions above are storage access paths. What the user types is Cypher, so
the compiler now decides between them.

`MATCH (a:P {val:7})-[:K*1..k]->(b:P) RETURN count(DISTINCT b)`, dense fixture,
compared against the same query with a relationship variable bound —
`-[e:K*1..k]->` — which makes path multiplicity observable and forces the old
path. Same binary, same session, same data:

| depth | rewritten | forced onto `og_vlp` |
|---|---|---|
| 2 | 2.39 ms | 2.08 ms *(both `og_vlp` — below the crossover)* |
| 3 | 36.61 ms | 36.19 ms *(both `og_vlp`)* |
| 4 | **204.43 ms** | 805.58 ms |
| 5 | **263.22 ms** | 17,714.73 ms |
| 8 | **277.39 ms** | does not finish |
| 20 | **260.83 ms** | does not finish |

The gap between these and the raw column above (263 ms against 59 ms at depth 5)
is the Cypher surface's own overhead, most of it building a jsonb object per
node so `count(DISTINCT b)` can compare whole nodes. That is the optimisation
target `docs/benchmark.md` already names, and it is untouched here.

---

## When the compiler rewrites, and when it must not

Two conditions, and being wrong about either changes answers rather than
timings.

**Is multiplicity observable?** `RETURN count(b)` counts walks; `RETURN
count(DISTINCT b)` counts nodes. The rewrite is allowed only when the query
cannot tell the difference:

- `RETURN DISTINCT …` — duplicate rows cannot survive it.
- otherwise the projection must aggregate, and every aggregate in it must ignore
  duplicates: `count(DISTINCT x)`, `collect(DISTINCT x)`, `min`, `max`. `count(x)`,
  `sum`, `avg` and anything user-defined disqualify it.
- a bound path variable (`MATCH p = …`) or relationship variable (`-[e:K*1..3]->`)
  disqualifies the hop regardless of what `RETURN` does — those *are* the paths.
- a `WITH` anywhere disqualifies the query, because it can aggregate before the
  `RETURN` and this pass does not look inside it.

**Is it worth it?** `og_reach` is a Rust set-returning function: unlike `og_vlp`
it does not inline into the surrounding plan, and it pays SPI setup per level.
Measured, that costs a few tenths of a millisecond — nothing at depth 5, but the
entire query at depth 2, where the first draft of this change made
`MATCH (a)-[:K*1..2]->(b) RETURN count(DISTINCT b)` **slower**. So the rewrite is
gated on the crossover being real: `Σ degreeⁱ > |V|`, with both terms read from
the planner's own statistics — a catalog lookup, not a scan. On the dense
fixture that says no at depth 3 (8,420 walks against 50,000 nodes) and yes at
depth 4 (168,420), which is exactly where the measurement puts the crossover.
Depth ≥ 12 skips the estimate: no plausible degree makes enumeration survivable
that deep. An unanalysed database has no statistics to answer with and falls
back to depth alone.

The regression suite asserts every one of these cases, in both directions, in
[`engine/tests/sql/05_reachability.sql`](../engine/tests/sql/05_reachability.sql):
that `count(DISTINCT y)` and `DISTINCT` rewrite, that `count(y)`, a bare
`RETURN y.name`, a path variable, a relationship variable and a shallow hop do
not, and that a graph with a cycle back to the start returns the same 4 nodes
either way while `count(y)` still returns its 6 trails.

---

## What the CSR is, and what it costs

`og_csr_build(etypes, dir)` compiles `og_data.og_adj` into backend-local memory:
a sorted `int8` id vector, and forward and reverse `u32` CSR arrays indexed by
position in it. `og_csr_reach` and `og_csr_hops` walk those; `og_csr_stats`
reports what a backend holds; `og_csr_drop` frees it.

Both directions are compiled because `og_adj` already stores both, so the
reverse costs no extra I/O — and a bidirectional shortest path is not correct
without it. `og_csr_hops` expands whichever frontier is smaller and finishes a
level before answering, so it reports the true shortest length rather than the
first meeting it finds.

The costs are exactly pgGraph's, and they are not small:

- **Per backend, not per database.** 119 ms and 8.4 MiB on the dense fixture,
  229 ms and 9.2 MiB on the sparse one — paid by every new connection that wants
  it. A connection-pooled service amortises that; a per-request-connection
  deployment pays it every request. This is the same effect visible in pgGraph's
  own published cold column, where every LDBC query lands between 2.8 and 3.4 s
  regardless of what was asked ([`comparison.md`](comparison.md)).
- **The snapshot is frozen at build time.** An edge committed after the build is
  invisible until it is rebuilt. There is no trigger capture here; pgGraph has
  one, and it is the obvious next thing if this path is kept.
- **Row-level security is not consulted.** A path through a row the caller may
  not read will appear in the result. `og_reach` has no such hole.
- **It cannot run in a parallel worker.** `og_reach` and the CSR functions are
  declared `PARALLEL RESTRICTED` — one uses SPI, the other backend-local state,
  and neither is available to a worker. A plan containing them runs in the
  leader.

Which is why the Cypher compiler routes to `og_reach` and not to the CSR. The
CSR is exposed, measured and documented; it is not silently substituted for a
query whose caller is entitled to MVCC and RLS.

---

## Against the other engines

The tables above compare this repository with itself. The obvious next question
is what the change is worth against Neo4j, Apache AGE and pgGraph — so all three
were installed and measured rather than quoted.

**pgGraph is in this one.** `docs/benchmark.md` excludes it because its published
figures come from a different dataset and machine with no correctness gate.
That objection does not apply to a build we compile ourselves and run on our own
data: pgGraph 1.1.0 was built from source against PostgreSQL 16 and installed
into the same server as Ontological and AGE. AGE 1.5.0 had to be built from
source too — the published `apache/age` image for this release is x86-64 only,
and measuring it under emulation on an arm64 host would have been a strawman.

### The question, and why it had to change

`count(DISTINCT b)` over `-[:K*1..k]->` is not a question every system states
identically. Cypher counts the start node again when a cycle returns to it;
pgGraph's `graph.traverse(include_start := false)` never does. That one-node
difference voids the comparison at exactly the depths being measured, so the
workload asks something unambiguous instead:

> distinct nodes **other than the start** within *k* hops

Every system expresses that exactly — `b.val <> a.val` in the three Cypher
dialects, `node <> start` in the CTE, `include_start := false` in pgGraph. All
seven systems returned identical answers at every depth either finished.

One cost of that normalisation has to be disclosed, because it is not evenly
distributed. Reading `b.val` in AGE is a JSON extraction per row, so the
exclusion predicate costs AGE far more than anyone else. Measured on the same
data, one hop:

| | classic `count(DISTINCT b)` | normalised, with `b.val <> a.val` |
|---|---|---|
| Ontological | 1.47 ms | 1.52 ms |
| Neo4j | 1.15 ms | 0.96 ms |
| Apache AGE | 1.44 ms | 94.00 ms |

So **AGE's column below overstates its traversal cost by roughly 90 ms**, and
its one-hop storage is competitive — the finding `docs/benchmark.md` already
reports. What the exclusion does not explain is the shape of its curve.

### 50,000 nodes / 974,936 edges, average degree 20

Median of five start nodes, warm, one PostgreSQL 16.14 with all three
extensions, Neo4j 5.26 over bolt, 60-second cap per statement.

| depth | Ontological (Cypher) | Ontological (`og_reach`) | recursive CTE | Apache AGE | pgGraph | Neo4j 5 |
|---|---|---|---|---|---|---|
| 1 | 1.52 | 0.11 | **0.08** | 94.00 | 1.16 | 0.96 |
| 2 | 2.64 | 0.18 | **0.19** | 761.43 | 10.00 | 2.94 |
| 3 | 29.06 | **1.06** | 2.17 | 13,696.72 | 237.87 | 4.66 |
| 4 | 193.35 | **18.96** | 27.53 | *>60 s* | 2,123.56 | 63.27 |
| 5 | 251.53 | **54.45** | 170.37 | — | 2,533.34 | 131.33 |
| 6 | 267.75 | **67.10** | 374.39 | — | 2,457.50 | 168.82 |
| 8 | 270.31 | **70.51** | 788.02 | — | 2,540.89 | 151.67 |
| property scan | 0.37 | 0.07 | **0.04** | — | 0.07 | 0.66 |

`—` means the system was not asked, because it had already blown the cap at a
shallower depth and the answer was known.

**AGE, asked as explicit fixed-length depths, is not in the table because it
took the server down.** At five and six spelled-out hops the backend was killed
by the kernel (`signal 9`) and PostgreSQL restarted, twice. That is a result —
it is what `docs/benchmark.md` recommends as the workaround for `*1..n`, and it
stops working somewhere past four hops on a million-edge graph — but a crashing
backend also voids every other system measured in the same run, which is why
the harness now detects a crash, stops asking that system anything deeper, and
waits for recovery before continuing.

### pgGraph: the traversal is fast, the row is expensive

Taking 2.4 s at face value would be the mistake this document keeps warning
about. The same traversal, at the same depth, asked for ten rows instead of all
of them:

| depth | traversal only (`max_rows := 10`) | full answer (~50,000 rows) |
|---|---|---|
| 2 | 1.67 ms | 10.00 ms |
| 4 | 31.52 ms | 2,123.56 ms |
| 6 | 42.76 ms | 2,457.50 ms |
| 8 | 42.46 ms | 2,540.89 ms |

**pgGraph's CSR walk goes flat at 42 ms and stays there to twenty hops**, which
is exactly the property its architecture claims. Everything above that is the
cost of materialising each reached node as a row: `graph.traverse` returns
eleven columns including three `jsonb` (`path`, `edge_path`, `node`), which is
about 48 µs per row and 2.4 s for fifty thousand of them. `hydrate := false`
does not change it, so it is not the source-row fetch.

This is a difference in what the API is for, not in engine quality. pgGraph's
`max_rows` defaults to 1,000 and its published figures are bounded traversals —
depth 1 and depth 2. Asked a bounded question it is quick: `graph.shortest_path`
over the same graph, four hops and five rows, is **8.0 ms**.

Two other measurements worth recording: the compiled artifact is **41.1 MB** for
this graph, against 8.4 MiB for the CSR in `og_csr_build` — pgGraph carries
labels, tenant bitmaps, active bits and sync overlays that ours does not — and
its own resource limits had to be raised (`graph.query_memory_mb`, 64 MB by
default) before a full 50,000-node traversal would run at all, for the same
reason AGE is given the indexes its documentation asks for.

### What the comparison actually says

- **Against AGE, the gap is now structural.** 471× at three hops on the
  normalised question, and AGE does not reach four. Subtract the 90 ms
  normalisation penalty and it is still 13.6 s against 1.06 ms.
- **Against Neo4j, this is the first workload where we win on depth.** Neo4j is
  clearly better from one to four hops — 4.66 ms at three hops against our
  29.06 ms through Cypher — and its curve also goes flat, at 151–169 ms. Our
  storage path is flat at 67–71 ms, so past five hops we are **2.3× faster**
  than Neo4j; through the Cypher surface we are 1.6× slower. The whole
  difference between those two statements is this repository's own query-engine
  overhead, which remains the largest single optimisation target in the codebase.
- **Against pgGraph, the architectures land within 1.6× of each other** on the
  traversal itself (42.8 ms against our 67.1 ms at six hops, both flat), and
  our compiled CSR is faster still at 4.86 ms. The 2.4 s figure is its result
  API, not its engine.
- **The recursive CTE is still the floor to one hop, and stops being it at six.**
  0.08 ms at one hop against our 0.11 ms; 788 ms at eight hops against our
  70.5 ms. Ten lines of SQL beat a graph database until the frontier saturates,
  and then they do not — which is a more useful thing to know than either
  system's headline number.

Reproduce with:

```bash
python3 bench/harness.py --scale 50000 --degree 20 --workload reach \
    --hops 1,2,3,4,5,6,8 --query-timeout 60 \
    --systems ontological,ontological_raw,cte,age,age_explicit,pggraph,neo4j
psql -d bench_pggraph -f bench/pggraph_cost.sql
```

Raw runs: `bench/results/bench-50000-20260817T033001Z.json` (the table above)
and `bench-50000-20260817T033525Z.json` (the classic-workload reference used for
the AGE normalisation penalty).

---

## What this does not show

- **One graph shape, uniformly random.** No hubs, no communities, no skew. Skew
  is what breaks frontier-based traversal, and a power-law fixture would test
  the frontier limits this branch does not have.
- **No concurrency.** Every query ran alone. The CSR's memory cost is per
  backend and its worst case is a connection storm; nothing here measures that.
- **One workload against the other engines.** Reachability counting, one graph
  shape, one scale. No pattern matching with predicates, no shortest path
  measured across all systems, no LDBC SNB, no concurrency, no writes.
- **pgGraph was measured on the question this document is about**, which is the
  one it can express — not on the bounded, paginated traversals its own
  benchmarks use and its API is shaped for. The split table above is there so
  that distinction is visible rather than buried in a single number.
- **No writes under measurement**, and no measurement of what a stale CSR costs
  to detect or rebuild.
- **`og_reach_sql` is not wired into anything.** It is in `access.sql` because
  it is the honest floor — the improvement available with no Rust at all — and
  because it makes the point that most of the win was algorithmic. The compiler
  does not emit it.

---

## Where this leaves the pgGraph comparison

The concession in [`comparison.md`](comparison.md) was that deep traversal is
structurally pgGraph's. Measured, that turns out to be two claims of very
different size:

| | factor at dense depth 6 |
|---|---|
| stop enumerating trails (`og_vlp` → `og_reach`), still in the heap | **691×** |
| leave the heap as well (`og_reach` → `og_csr_reach`) | **15×** |

The first was ours to fix and cost no guarantees. The second is the
architectural trade pgGraph makes, it is real, and it buys about one order of
magnitude — for a frozen snapshot, no RLS, and a per-backend compile.

Forking pgGraph would have bought the 15× and none of the 691×, because the
691× was not in its problem domain: our recursive CTE was answering a harder
question than anyone asked.
