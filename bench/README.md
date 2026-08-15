# Benchmarks

The claim this project rests on is that Apache AGE's storage and query design
give up more performance than they need to. That claim is only worth as much as
its evidence, so the harness is built to be hostile to its own conclusions.

## Method

**Same data, same machine.** One deterministic edge generator produces the
graph; each system loads it through its own bulk path. The PostgreSQL-based
systems share one container; Neo4j and TypeDB run as their own servers on the
same host, reached over published ports like the PostgreSQL one. Nothing is
compared across machines.

**Answers are checked before timings are reported.** Every system runs the same
logical queries; if the results differ, the harness prints the disagreement and
marks those timings **void**. This is not a formality — it caught a real
mismatch during development (the systems were starting from different nodes
because a `val` property was not unique), and every number before that fix was
meaningless.

**Timing happens on one already-open connection.** Spawning a process per query
costs about 12 ms, which is more than the queries being compared; measuring that
way makes every system look identical. `\timing` reports per-statement time on
one psql connection; Neo4j and TypeDB are timed client-side on a reused session
with the result consumed, which is the same quantity.

**The protocol floor is published too.** A trivial query costs 0.15 ms through
psql and 1.6 ms through the Python bolt driver. That difference is a large part
of a one-hop answer, so every result file records each system's floor next to
its timings — read the two together or Neo4j looks slower than it is.

**Every distinct query text is warmed.** Each query is issued from five
different start nodes, and warm-up covers all five, so no system is charged for
a cold plan cache on a text another system had already planned.

**Page counts are reported next to latency.** Latency moves with cache state;
logical page accesses are a direct function of the storage layout, so they are
the honest measure of a storage claim.

**Two Ontological rows.** `ontological` is the user-facing Cypher path — the
right comparison against AGE's `cypher()`. `ontological_raw` is the storage
access path. The gap between them is the query engine's overhead, and publishing
both keeps that cost visible instead of hidden.

## Running

```bash
python3 bench/harness.py                                   # default 20k nodes
python3 bench/harness.py --scale 50000 --degree 20 --runs 8
python3 bench/harness.py --systems ontological,cte         # skip AGE
python3 bench/harness.py --compare-baseline bench/results/baseline.json
```

Apache AGE must be installed for the `age` system; the harness skips it with a
notice rather than failing. Results are written to `bench/results/` as JSON.

`neo4j` and `typedb` need a server and a driver — both are skipped with a notice
when either is missing:

```bash
docker run -d --name bench-neo4j  -p 27687:7687 \
  -e NEO4J_AUTH=neo4j/benchpass123 neo4j:5
docker run -d --name bench-typedb -p 21729:1729 typedb/typedb:latest
pip install neo4j typedb-driver
```

Connection settings come from `NEO4J_URI` / `NEO4J_USER` / `NEO4J_PASSWORD` and
`TYPEDB_ADDR` / `TYPEDB_USER` / `TYPEDB_PASSWORD`; the defaults match the two
commands above. The full cross-product write-up lives in
[docs/benchmark.md](../docs/benchmark.md).

The Studio serves these result files as a report page at
`http://localhost:7474/benchmark.html`. It reads this directory directly and
takes the newest run per system, so a fresh harness run is published by the act
of finishing — no number is ever copied into the page by hand.

## Results, 2026-08-06

50,000 nodes / 974,936 edges, median latency. All systems returned identical
answers; integrity violations: 0. Full write-up, method and the 5,000-node
tables: [docs/benchmark.md](../docs/benchmark.md).

| query | ontological | ontological_raw | neo4j | age | age_explicit | typedb | cte |
|---|---|---|---|---|---|---|---|
| 1 hop | 2.10 ms | 0.31 ms | 0.74 ms | 2.15 ms | 2.15 ms | 0.46 ms | 0.24 ms |
| 2 hops | 3.96 ms | 0.50 ms | 0.92 ms | 799.50 ms | 7.99 ms | 1.48 ms | 0.36 ms |
| 3 hops | 33.86 ms | 4.33 ms | 2.99 ms | 22,412 ms | 34.63 ms | 19.73 ms | 3.49 ms |
| property scan | 0.62 ms | 0.24 ms | 0.68 ms | 0.27 ms | 0.24 ms | 0.52 ms | 0.19 ms |
| *protocol floor* | *0.19 ms* | *0.19 ms* | *0.79 ms* | *0.17 ms* | *0.18 ms* | *0.37 ms* | *0.18 ms* |

### What the numbers say against us

- **The recursive CTE still wins nearly every cell.** Two indexed tables and no
  query engine is very hard to beat on a workload this shape. Anything in this
  table has to justify itself against that, not against the other products.
- **Neo4j is 11× faster at three hops** (2.99 ms vs 33.86 ms) and its latency
  barely moves with scale. This is the number to beat.
- **The Cypher surface costs 7.8× over raw storage at three hops** (33.86 ms vs
  4.33 ms). jsonb projection and SPI round-tripping dominate. It is the clearest
  optimisation target in the codebase.
- **The AGE comparison is narrower than it looks.** AGE collapses on `*1..n`,
  not on storage: asked the same question as a union of fixed-length patterns,
  it answers in 34.63 ms instead of 22,412 ms. Against `age_explicit` this
  project is roughly at parity at three hops, not 660× ahead. Any claim made
  from the `age` column alone is a claim about AGE's variable-length path
  operator.

## Regression gate

```bash
python3 bench/harness.py --compare-baseline bench/results/baseline.json
```

Exits non-zero when any query is more than 20% slower than the baseline, and
prints which ones. `bench/results/baseline.json` is committed; update it
deliberately, never as a side effect. It was last refreshed on 2026-08-06, when
the harness started building the property and edge-endpoint indexes described
above — the previous baseline measured an unindexed AGE and an unindexed
Ontological, so its numbers are not comparable to anything produced since.

## Not yet implemented

- LDBC SNB (spec 009 phase 2) — the generator is not wired up
- openCypher TCK pass-rate tracking (phase 1)
- Fault injection and long-running stress (phase 4)
- Concurrency: every query in these results ran alone
- A recursive-function variant of the TypeDB traversal — the measured one is the
  hand-written disjunction, which re-walks its own prefixes

Those are gaps in the harness, not results that were run and omitted.
