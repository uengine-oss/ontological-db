# Deep-traversal bench

What multi-hop traversal costs here, and what leaving the heap would buy.

Four ways to answer *"which nodes are within k hops of this one"*, measured on
the same data in the same server, with every answer checked against the others
before any timing is reported:

| variant | function | where the work happens |
|---|---|---|
| `vlp` | `og_vlp()` | recursive CTE, enumerates trails, carries an `int8[]` path |
| `reach_sql` | `og_reach_sql()` | same CTE with no path array and `UNION` instead of `UNION ALL` |
| `reach` | `og_reach()` | Rust BFS with a visited set, adjacency read through SPI |
| `csr` | `og_csr_reach()` | backend-local compiled CSR, no SPI, no heap, no planner |

`reach` keeps every property the heap gives it — MVCC, row-level security, this
transaction's own uncommitted writes. `csr` is the pgGraph shape and gives all
three up in exchange for the hot loop.

## Running it

```bash
# fixtures
createdb bench_csr    && psql -d bench_csr    -v nodes=50000  -v degree=20 -f gen.sql
createdb bench_sparse && psql -d bench_sparse -v nodes=200000 -v degree=4  -f gen.sql

# the sweep — every variant, every depth, answers compared
python3 deep.py --db bench_csr    --depths 1,2,3,4,5,6 --starts 5 --label dense
python3 deep.py --db bench_csr    --depths 7,8,10,16,20 --variants reach_sql,reach,csr --label dense-deep
python3 deep.py --db bench_sparse --depths 1,2,3,4,5,6 --starts 5 --label sparse
python3 deep.py --db bench_sparse --depths 8,10,12,16,20 --variants reach_sql,reach,csr --label sparse-deep

# end-to-end Cypher, rewritten vs not, inside one binary
psql -d bench_csr -f cypher_ab.sql
```

`deep.py` writes one JSON per run into `results/`, including the answers each
variant gave, so a disagreement is visible in the record and not only on the
terminal. It exits non-zero if any variant disagreed.

Timings assume the server the repository's own harness assumes —
`PGHOST=localhost PGPORT=28816`, the port `start.sh` brings up.

Findings and what to do about them: [`docs/deep-traversal.md`](../../docs/deep-traversal.md).
