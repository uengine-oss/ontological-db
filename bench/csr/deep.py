#!/usr/bin/env python3
"""
Deep-traversal bench: what does multi-hop actually cost, and what would leaving
the heap buy?

Four ways to answer "which nodes are within k hops of this one", on the same
data, in the same server, with every answer checked against the others before a
single timing is reported:

  vlp        og_vlp()        — today's path. Enumerates trails, carries an
                               int8[] path, so the row count is degree^k.
  reach_sql  og_reach_sql()  — same recursive CTE with no path array and UNION
                               instead of UNION ALL. Pure SQL, no new code.
  reach      og_reach()      — Rust BFS with a visited set, adjacency read
                               through SPI. Still MVCC, still RLS, still sees
                               this transaction's writes.
  csr        og_csr_reach()  — the pgGraph shape. Topology compiled once into a
                               backend-local CSR of u32 indices, walked with no
                               SPI and no planner. Frozen snapshot, no RLS.

Usage
  python3 bench/csr/deep.py --db bench_csr --depths 1,2,3,4,5,6
  python3 bench/csr/deep.py --db bench_sparse --depths 1,3,5,7,9,11 --timeout 120

The fixture is built by gen.sql:
  psql -d bench_csr -v nodes=50000 -v degree=20 -f bench/csr/gen.sql
"""

import argparse
import json
import re
import statistics
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
RESULTS = HERE / "results"

PSQL = "psql"
PGHOST = "localhost"
PGPORT = "28816"

# Every variant answers the same question: how many distinct nodes lie within
# `d` hops. `vlp` needs count(DISTINCT) because it returns one row per trail;
# the others return one row per node and must not be given that crutch, since
# deduplicating for them is precisely the work being measured.
VARIANTS = {
    "vlp":       "SELECT count(DISTINCT node) FROM og_vlp({s}, ARRAY[{r}]::int4[], 'o', 1, {d})",
    "reach_sql": "SELECT count(*) FROM og_reach_sql({s}, ARRAY[{r}]::int4[], 'o', 1, {d})",
    "reach":     "SELECT count(*) FROM og_reach({s}, ARRAY[{r}]::int4[], 'o', 1, {d})",
    "csr":       "SELECT count(*) FROM og_csr_reach({s}, 1, {d})",
}
# csr must find its compiled graph already in the backend; building it is timed
# separately, because that cost is the architecture's, not the query's.
PRELUDE = {"csr": "SELECT * FROM og_csr_build(ARRAY[{r}]::int4[], 'o')"}


def run_psql(db, sql, timeout_s):
    args = [PSQL, "-h", PGHOST, "-p", PGPORT, "-d", db, "-q", "-tA", "-f", "-"]
    return subprocess.run(args, input=sql, capture_output=True, text=True,
                          timeout=timeout_s + 60)


def session(db, statements, timeout_s, prelude=None, warmup=1):
    """
    Time statements inside one connection.

    A psql per query costs ~12 ms, which is more than most of what is being
    measured here; `\\timing` inside one session measures the server instead.
    The prelude is deliberately outside the timing — for `csr` it is the
    backend-local compile, whose cost is reported on its own.
    """
    lines = ["\\set ON_ERROR_STOP on", f"SET statement_timeout = '{timeout_s}s';"]
    if prelude:
        lines.append(prelude.rstrip(";") + ";")
    lines.append("\\timing on")
    # Warm-up first, then every statement once in order, so timings and answers
    # line up with the start nodes that produced them.
    seq = statements[:1] * warmup + statements
    lines += [q.rstrip(";") + ";" for q in seq]

    r = run_psql(db, "\n".join(lines), timeout_s * (warmup + len(statements)) + 120)
    if r.returncode != 0:
        err = r.stderr.strip()
        if "statement timeout" in err or "canceling statement" in err:
            return None, None
        raise RuntimeError(err[:2000])
    times = [float(m) for m in re.findall(r"^Time: ([\d.]+) ms", r.stdout, re.M)][warmup:]
    answers = [l for l in r.stdout.splitlines() if re.fullmatch(r"\d+", l.strip())][warmup:]
    if not times:
        raise RuntimeError("no timings captured")
    return times, answers


def summarise(times):
    times = sorted(times)
    return {
        "median_ms": round(statistics.median(times), 3),
        "min_ms": round(times[0], 3),
        "max_ms": round(times[-1], 3),
        "runs": len(times),
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", default="bench_csr")
    ap.add_argument("--depths", default="1,2,3,4,5,6")
    ap.add_argument("--starts", type=int, default=5)
    ap.add_argument("--timeout", type=int, default=120,
                    help="per-statement timeout, seconds; a variant that hits it is void")
    ap.add_argument("--variants", default=",".join(VARIANTS))
    ap.add_argument("--label", default="")
    args = ap.parse_args()

    depths = [int(d) for d in args.depths.split(",")]
    variants = [v.strip() for v in args.variants.split(",")]

    tid = run_psql(args.db, "SELECT og_type_id('benchg','P')", 30).stdout.strip()
    rid = run_psql(args.db, "SELECT og_type_id('benchg','K')", 30).stdout.strip()
    n_nodes = int(run_psql(args.db, "SELECT count(*) FROM og_data.og_node", 120).stdout)
    # Fixed start nodes, spread evenly across the id space, same for every variant.
    vals = ",".join(str(i * (n_nodes // (args.starts + 1))) for i in range(1, args.starts + 1))
    starts = run_psql(args.db, f"""
        SELECT id FROM og_data.n_{tid} WHERE p_val IN ({vals}) ORDER BY id""", 60).stdout.split()
    shape = run_psql(args.db, """
        SELECT (SELECT count(*) FROM og_data.og_node) || '/' ||
               (SELECT count(*) FROM og_data.og_edge)""", 120).stdout.strip()

    build = run_psql(args.db, f"SELECT * FROM og_csr_build(ARRAY[{rid}]::int4[], 'o')", 600)
    nodes, edges, nbytes, build_ms = build.stdout.strip().split("|")

    print(f"db={args.db}  graph={shape}  starts={len(starts)}  timeout={args.timeout}s")
    print(f"csr compile: {int(nodes):,} nodes / {int(edges):,} edges, "
          f"{int(nbytes)/2**20:.1f} MiB, {float(build_ms):.1f} ms per backend\n")

    out = {
        "when": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "db": args.db, "label": args.label, "graph": shape,
        "csr_build": {"nodes": int(nodes), "edges": int(edges),
                      "bytes": int(nbytes), "ms": round(float(build_ms), 1)},
        "starts": starts, "depths": depths, "cells": {},
    }

    header = f"{'depth':>6} " + "".join(f"{v:>14}" for v in variants)
    print(header)
    print("-" * len(header))

    # Answers a *reachable* variant already agreed on, keyed by (depth, start).
    truth = {}
    for d in depths:
        row = f"{d:>6} "
        for v in variants:
            sqls = [VARIANTS[v].format(s=s, r=rid, d=d) for s in starts]
            prelude = PRELUDE.get(v, "").format(r=rid) or None
            try:
                times, answers = session(args.db, sqls, args.timeout, prelude)
            except RuntimeError as e:
                out["cells"][f"{v}@{d}"] = {"error": str(e)[:200]}
                row += f"{'error':>14}"
                continue
            if times is None:
                out["cells"][f"{v}@{d}"] = {"timeout_s": args.timeout}
                row += f"{'>%ds' % args.timeout:>14}"
                continue

            agree = True
            for s, a in zip(starts, answers):
                prev = truth.setdefault((d, s), a)
                if prev != a:
                    agree = False
            cell = summarise(times)
            cell["answers"] = answers
            cell["agrees"] = agree
            out["cells"][f"{v}@{d}"] = cell
            mark = "" if agree else " !"
            row += f"{cell['median_ms']:>13.2f}{mark or ' '}"
        print(row)

    bad = [k for k, c in out["cells"].items() if c.get("agrees") is False]
    print()
    if bad:
        print(f"DISAGREEMENT in {bad} — those timings are void")
    else:
        print("all variants returned identical answers at every depth measured")

    RESULTS.mkdir(exist_ok=True)
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    path = RESULTS / f"deep-{args.label or args.db}-{stamp}.json"
    path.write_text(json.dumps(out, indent=2))
    print(f"\nwrote {path}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
