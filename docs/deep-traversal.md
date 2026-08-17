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

## What this does not show

- **One graph shape, uniformly random.** No hubs, no communities, no skew. Skew
  is what breaks frontier-based traversal, and a power-law fixture would test
  the frontier limits this branch does not have.
- **No concurrency.** Every query ran alone. The CSR's memory cost is per
  backend and its worst case is a connection storm; nothing here measures that.
- **No comparison against Neo4j, AGE or pgGraph.** The systems in
  `docs/benchmark.md` were not re-run. The `og_vlp` column is this repository's
  own previous behaviour, which is the only baseline this change is entitled to
  claim against.
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
