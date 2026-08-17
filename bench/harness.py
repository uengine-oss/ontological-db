#!/usr/bin/env python3
"""
Ontological benchmark harness — spec 009.

Runs the same workload against several systems in the same container on the
same data, checks that they return the *same answers*, and only then reports
timings. A benchmark that skips the correctness check is marketing.

Systems
  ontological  — this project
  age          — Apache AGE, the incumbent Cypher-on-PostgreSQL extension
  cte          — hand-written recursive CTE over plain tables (the honest floor)
  neo4j        — Neo4j 5, the native property-graph incumbent (bolt, separate server)
  typedb       — TypeDB 3, the typed/ontological incumbent (separate server)

Usage
  python3 bench/harness.py --scale 10000 --systems ontological,age,cte
  python3 bench/harness.py --systems neo4j,typedb --scale 5000
  python3 bench/harness.py --compare-baseline results/baseline.json

neo4j and typedb need their drivers (`pip install neo4j typedb-driver`) and a
running server; the harness skips them with a notice when either is missing.
Connection settings come from the environment:

  NEO4J_URI   (default bolt://localhost:27687)   NEO4J_USER / NEO4J_PASSWORD
  TYPEDB_ADDR (default localhost:21729)          TYPEDB_USER / TYPEDB_PASSWORD
"""

import argparse
import itertools
import json
import os
import random
import statistics
import re
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
RESULTS = HERE / "results"

PSQL = os.environ.get("OG_PSQL", "psql")
PGHOST = os.environ.get("PGHOST", "localhost")
PGPORT = os.environ.get("PGPORT", "28816")


# --------------------------------------------------------------------------
# plumbing
# --------------------------------------------------------------------------

def psql(db, sql, tuples_only=True, timeout=600):
    """
    Statements go in on stdin, not argv: bulk-load SQL is megabytes long and
    argv is not.
    """
    args = [PSQL, "-h", PGHOST, "-p", PGPORT, "-d", db, "-v", "ON_ERROR_STOP=1", "-q"]
    if tuples_only:
        args += ["-tA"]
    args += ["-f", "-"]
    r = subprocess.run(args, input=sql, capture_output=True, text=True, timeout=timeout)
    if r.returncode != 0:
        raise RuntimeError(r.stderr.strip()[:4000])
    return r.stdout.strip()


def psql_file(db, path, timeout=1800):
    r = subprocess.run(
        [PSQL, "-h", PGHOST, "-p", PGPORT, "-d", db, "-v", "ON_ERROR_STOP=1", "-q", "-f", str(path)],
        capture_output=True, text=True, timeout=timeout,
    )
    if r.returncode != 0:
        raise RuntimeError(r.stderr.strip()[:4000])
    return r.stdout


def recreate(db):
    for stmt in (f'DROP DATABASE IF EXISTS "{db}"', f'CREATE DATABASE "{db}"'):
        r = subprocess.run(
            [PSQL, "-h", PGHOST, "-p", PGPORT, "-d", "postgres", "-q", "-c", stmt],
            capture_output=True, text=True, timeout=300)
        if r.returncode != 0:
            raise RuntimeError(r.stderr.strip()[:2000])


class Timeout(Exception):
    """A system was given the same wall-clock budget as everyone else and used it."""

    def __init__(self, seconds):
        super().__init__(f"exceeded {seconds}s")
        self.seconds = seconds


class Crashed(Exception):
    """
    A query took the server down with it.

    This is a result, not an accident: a deep enough question asked the wrong
    way gets the backend killed by the kernel, and that is worth reporting. But
    a crashing backend restarts the whole postmaster, so without handling it
    here one system's failure silently voids every other system's numbers in
    the same run — which is exactly what it did the first time.
    """


CRASH_SIGNS = (
    "server closed the connection",
    "terminated by signal",
    "in recovery mode",
    "not yet accepting connections",
    "crash of another server process",
    "connection to server was lost",
)


def looks_like_crash(err):
    return any(sign in err for sign in CRASH_SIGNS)


def wait_for_server(seconds=180):
    """Block until the server is accepting connections again."""
    deadline = time.time() + seconds
    while time.time() < deadline:
        r = subprocess.run(
            [PSQL, "-h", PGHOST, "-p", PGPORT, "-d", "postgres", "-tAc", "SELECT 1"],
            capture_output=True, text=True, timeout=30)
        if r.returncode == 0:
            return True
        time.sleep(2)
    return False


def timed_in_session(db, prelude, sqls, runs, warmup=None, timeout_s=None):
    """
    Time queries inside a single psql session.

    Spawning psql per query costs ~12 ms — an order of magnitude more than the
    queries we are comparing, which would make every system look identical.
    `\\timing` reports per-statement time within one connection instead.

    Warm-up covers every distinct query text at least once, so no system is
    charged for a cold plan cache on a text another system had already seen.
    """
    warmup = max(2, len(sqls)) if warmup is None else warmup
    lines = ["\\timing off"]
    if timeout_s:
        # A deep hop on a system that enumerates paths does not return in this
        # lifetime. The cap is the same for everyone and is recorded next to
        # the result, so "did not finish" is a measurement rather than a hang.
        lines.append(f"SET statement_timeout = '{int(timeout_s)}s';")
    if prelude:
        lines.append(prelude.rstrip().rstrip(";") + ";")
    lines.append("\\timing on")
    seq = []
    for i in range(warmup + runs):
        seq.append(sqls[i % len(sqls)])
    lines += [q.rstrip().rstrip(";") + ";" for q in seq]

    args = [PSQL, "-h", PGHOST, "-p", PGPORT, "-d", db, "-v", "ON_ERROR_STOP=1", "-q", "-tA", "-f", "-"]
    wall = (timeout_s + 30) * (warmup + runs) if timeout_s else 1800
    r = subprocess.run(args, input="\n".join(lines), capture_output=True, text=True, timeout=wall)
    if r.returncode != 0:
        err = r.stderr.strip()
        if "statement timeout" in err or "canceling statement" in err:
            raise Timeout(timeout_s)
        if looks_like_crash(err):
            raise Crashed(err[:400])
        raise RuntimeError(err[:2000])
    times = [float(m) for m in re.findall(r"^Time: ([\d.]+) ms", r.stdout, re.M)]
    times = times[warmup:]
    if not times:
        raise RuntimeError("no timings captured")
    times.sort()
    return {
        "median_ms": round(statistics.median(times), 3),
        "p95_ms": round(times[min(len(times) - 1, int(len(times) * 0.95))], 3),
        "min_ms": round(times[0], 3),
        "runs": len(times),
    }


def buffers_read(db, sql, prelude=""):
    """
    Logical page accesses for a statement.

    Latency moves with cache state; page count is a direct function of the
    storage layout, so this is the honest measure of spec 001's claim.
    """
    script = ((prelude.rstrip().rstrip(";") + ";\n") if prelude else "") + \
             f"EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {sql}"
    out = psql(db, script)
    out = out[out.index("["):]
    plan = json.loads(out)[0]["Plan"]
    total = 0

    def walk(n):
        nonlocal total
        total += n.get("Shared Hit Blocks", 0) + n.get("Shared Read Blocks", 0)
        for c in n.get("Plans", []):
            walk(c)

    walk(plan)
    return total


# --------------------------------------------------------------------------
# data generation — one deterministic graph, three physical encodings
# --------------------------------------------------------------------------

def gen_edges(n_nodes, avg_degree, seed=42, shape="random"):
    """
    One deterministic edge list, in one of three shapes.

    `random` is the shape every table in `docs/benchmark.md` uses, and it is the
    wrong instrument for a question about depth: at average degree 20 the whole
    graph sits inside five hops, so "twenty hops" and "eight hops" are the same
    question asked twice. Depth only means something when the diameter is large,
    and these are the two shapes where it is:

      chain — a line. Diameter |V|, out-degree 1, exactly one path anywhere.
              Lineage, provenance, a supply chain, a reply thread. Isolates
              per-hop overhead from frontier work.
      grid  — a square lattice pointing right and down. Diameter 2(√|V| - 1),
              frontier grows linearly, nodes-within-k grows as k²/2 — but the
              number of *paths* to (i,j) is C(i+j, i), which is combinatorial.
              Road networks, meshes, dependency DAGs.
    """
    if shape == "chain":
        return [(i, i + 1) for i in range(n_nodes - 1)]
    if shape == "grid":
        side = int(round(n_nodes ** 0.5))
        n_nodes = side * side
        edges = []
        for i in range(n_nodes):
            if i % side < side - 1:
                edges.append((i, i + 1))
            if i + side < n_nodes:
                edges.append((i, i + side))
        return edges
    rnd = random.Random(seed)
    edges = []
    for src in range(n_nodes):
        for _ in range(max(1, int(rnd.gauss(avg_degree, avg_degree / 3)))):
            dst = rnd.randrange(n_nodes)
            if dst != src:
                edges.append((src, dst))
    return edges


class PgSystem:
    """
    Shared behaviour for everything that lives inside the benchmark's
    PostgreSQL: one psql session per measurement, `\\timing` for latency,
    EXPLAIN (BUFFERS) for page counts.
    """
    prelude = ""
    engine = None
    floor_query = "SELECT 1"
    reuses = None  # set when this system reads a database another system loaded

    def version(self):
        return psql("postgres", "SELECT version()").split(" on ")[0]

    def answer(self, query, timeout_s=None):
        pre = (self.prelude.rstrip().rstrip(";") + ";\n") if self.prelude else ""
        if timeout_s:
            pre = f"SET statement_timeout = '{int(timeout_s)}s';\n" + pre
        try:
            out = psql(self.db, pre + query, timeout=(timeout_s or 600) + 60)
        except RuntimeError as e:
            if looks_like_crash(str(e)):
                raise Crashed(str(e)[:400])
            raise
        raw = out.strip().splitlines()[-1].strip()
        # og_cypher -> {"count(b)": 8}; AGE -> 8; plain SQL -> 8
        if raw.startswith("{"):
            raw = str(list(json.loads(raw).values())[0])
        return raw.strip('"')

    def measure(self, queries, runs, warmup=None, timeout_s=None):
        m = timed_in_session(self.db, self.prelude, queries, runs, warmup, timeout_s)
        try:
            m["buffers"] = buffers_read(self.db, queries[0], self.prelude)
        except Exception:
            pass
        return m

    def reach_hop(self, start_local, hops):
        """
        Distinct nodes *other than the start* within `hops` hops.

        The deep workload needs a question every system can state exactly.
        `n_hop` cannot be it: Cypher counts the start node again when a cycle
        returns to it, and pgGraph's `graph.traverse(include_start := false)`
        never does — a one-node difference that voids the comparison at exactly
        the depths it is supposed to measure. Excluding the start everywhere
        removes the ambiguity without favouring anyone; `val` is unique per
        node, so `b.val <> a.val` is node identity in every dialect here.
        """
        raise NotImplementedError

    def teardown(self):
        pass


class Ontological(PgSystem):
    name = "ontological"
    db = "bench_og"
    prelude = ""

    def setup(self, n_nodes, edges):
        recreate(self.db)
        psql(self.db, "CREATE EXTENSION ontological CASCADE", tuples_only=False)
        psql(self.db, "SELECT og_create_graph('benchg')")
        psql(self.db, "SELECT og_create_type('benchg','P','entity')")
        psql(self.db, "SELECT og_add_property('benchg','P','name','string')")
        psql(self.db, "SELECT og_add_property('benchg','P','val','int')")
        psql(self.db, "SELECT og_create_type('benchg','K','relation')")

        # Bulk load through SQL rather than one Cypher CREATE per row: the
        # per-statement overhead would dominate and tell us nothing.
        tid = int(psql(self.db, "SELECT og_type_id('benchg','P')"))
        rid = int(psql(self.db, "SELECT og_type_id('benchg','K')"))
        self.tid, self.rid = tid, rid
        psql(self.db, f"""
            INSERT INTO og_data.og_node (id, type_id)
            SELECT og_make_id(0,{tid},i+1), {tid} FROM generate_series(0,{n_nodes-1}) i;
            INSERT INTO og_data.n_{tid} (id, p_name, p_val)
            SELECT og_make_id(0,{tid},i+1), 'n'||i, i FROM generate_series(0,{n_nodes-1}) i;
        """.replace("%%", "%"), tuples_only=False)

        values = ",".join(f"({s},{d})" for s, d in edges)
        psql(self.db, f"""
            CREATE TEMP TABLE e_raw(s int, d int);
            INSERT INTO e_raw VALUES {values};
            INSERT INTO og_data.og_edge (id, type_id, src, dst)
            SELECT og_make_id(0,{rid}, row_number() OVER ()), {rid},
                   og_make_id(0,{tid}, s+1), og_make_id(0,{tid}, d+1) FROM e_raw;
            INSERT INTO og_data.e_{rid} (id, src, dst)
            SELECT id, src, dst FROM og_data.og_edge WHERE type_id = {rid};
            INSERT INTO og_data.og_adj (src, etype, dir, seq, n, nbr, eid)
            SELECT src, {rid}, 'o', chunk, count(*)::int4, array_agg(dst), array_agg(id)
              FROM (SELECT src, dst, id,
                           ((row_number() OVER (PARTITION BY src ORDER BY id)) - 1)::int4 / 256 AS chunk
                      FROM og_data.og_edge WHERE type_id = {rid}) x
             GROUP BY src, chunk;
            INSERT INTO og_data.og_adj (src, etype, dir, seq, n, nbr, eid)
            SELECT dst, {rid}, 'i', chunk, count(*)::int4, array_agg(src), array_agg(id)
              FROM (SELECT src, dst, id,
                           ((row_number() OVER (PARTITION BY dst ORDER BY id)) - 1)::int4 / 256 AS chunk
                      FROM og_data.og_edge WHERE type_id = {rid}) x
             GROUP BY dst, chunk;
        """, tuples_only=False)
        # The start property is looked up by every query, so it gets the index
        # a competent operator would create. Every system in the comparison is
        # indexed the same way for the same reason.
        psql(self.db, "SELECT og_create_index('benchg','P','val'); ANALYZE;",
             tuples_only=False)

    def one_hop(self, start_local):
        return (f"SELECT og_cypher('benchg', $$ MATCH (a:P)-[:K]->(b:P) "
                f"WHERE a.val = {start_local} RETURN count(b) $$)")

    def n_hop(self, start_local, hops):
        return (f"SELECT og_cypher('benchg', $$ MATCH (a:P)-[:K*1..{hops}]->(b:P) "
                f"WHERE a.val = {start_local} RETURN count(DISTINCT b) $$)")

    def reach_hop(self, start_local, hops):
        return (f"SELECT og_cypher('benchg', $$ MATCH (a:P)-[:K*1..{hops}]->(b:P) "
                f"WHERE a.val = {start_local} AND b.val <> {start_local} "
                f"RETURN count(DISTINCT b) $$)")

    def prop_scan(self):
        return ("SELECT og_cypher('benchg', $$ MATCH (a:P) WHERE a.val < 100 "
                "RETURN count(a) $$)")


class OntologicalRaw(Ontological):
    """
    Same data, but through the storage access paths instead of the Cypher
    surface. The gap between this and `ontological` is the query engine's
    overhead; the gap between this and `age` is the storage design.
    """
    name = "ontological_raw"
    db = "bench_og"
    prelude = ""
    reuses = "ontological"

    def setup(self, n_nodes, edges):
        # Reuses the database Ontological already built.
        self.tid = int(psql(self.db, "SELECT og_type_id('benchg','P')"))
        self.rid = int(psql(self.db, "SELECT og_type_id('benchg','K')"))

    def _start(self, start_local):
        return f"(SELECT id FROM og_data.n_{self.tid} WHERE p_val = {start_local})"

    def one_hop(self, start_local):
        return (f"SELECT count(*) FROM og_expand({self._start(start_local)}, "
                f"ARRAY[{self.rid}]::int4[], 'o')")

    def reach_hop(self, start_local, hops):
        # Inherited from Ontological this would have measured the Cypher
        # surface twice. The point of this row is the storage access path.
        return (f"SELECT count(*) FROM og_reach({self._start(start_local)}, "
                f"ARRAY[{self.rid}]::int4[], 'o'::\"char\", 1, {hops}) r "
                f"WHERE r.node <> {self._start(start_local)}")

    def n_hop(self, start_local, hops):
        return (f"SELECT count(DISTINCT node) FROM og_vlp({self._start(start_local)}, "
                f"ARRAY[{self.rid}]::int4[], 'o'::\"char\", 1, {hops})")

    def prop_scan(self):
        return f"SELECT count(*) FROM og_data.n_{self.tid} WHERE p_val < 100"


class AGE(PgSystem):
    name = "age"
    db = "bench_age"
    prelude = "LOAD 'age'; SET search_path = ag_catalog, public"

    def version(self):
        try:
            return "Apache AGE " + psql(
                "postgres", "SELECT default_version FROM pg_available_extensions "
                            "WHERE name='age'")
        except Exception:
            return "Apache AGE"

    def available(self):
        try:
            psql("postgres", "SELECT 1 FROM pg_available_extensions WHERE name='age'")
            return psql("postgres",
                        "SELECT count(*) FROM pg_available_extensions WHERE name='age'") == "1"
        except Exception:
            return False

    def setup(self, n_nodes, edges):
        recreate(self.db)
        psql(self.db, "CREATE EXTENSION age", tuples_only=False)
        psql(self.db, "LOAD 'age'; SET search_path = ag_catalog, public; "
                      "SELECT create_graph('benchg')", tuples_only=False)
        # AGE's own bulk path: insert straight into the label tables.
        psql(self.db, f"""
            LOAD 'age'; SET search_path = ag_catalog, public;
            SELECT create_vlabel('benchg','P'); SELECT create_elabel('benchg','K');
            INSERT INTO benchg."P" (properties)
            SELECT agtype_build_map('name', 'n'||i, 'val', i)
              FROM generate_series(0,{n_nodes-1}) i;
        """.replace("%%", "%"), tuples_only=False)
        values = ",".join(f"({s},{d})" for s, d in edges)
        psql(self.db, f"""
            LOAD 'age'; SET search_path = ag_catalog, public;
            CREATE TEMP TABLE e_raw(s int, d int);
            INSERT INTO e_raw VALUES {values};
            CREATE TEMP TABLE idx AS
              SELECT id, row_number() OVER (ORDER BY id) - 1 AS ord FROM benchg."P";
            CREATE INDEX ON idx(ord);
            INSERT INTO benchg."K" (start_id, end_id, properties)
            SELECT a.id, c.id, agtype_build_map()
              FROM e_raw r JOIN idx a ON a.ord = r.s JOIN idx c ON c.ord = r.d;
        """, tuples_only=False)
        # AGE creates no index beyond the primary key on `id` — not on the
        # edge endpoints, not on properties. Benchmarking it that way would be
        # a strawman, so it gets the three indexes its documentation tells you
        # to create, and they are built before anything is timed.
        psql(self.db, """
            LOAD 'age'; SET search_path = ag_catalog, public;
            CREATE INDEX k_start ON benchg."K" (start_id);
            CREATE INDEX k_end   ON benchg."K" (end_id);
            CREATE INDEX p_val   ON benchg."P"
                (agtype_access_operator(VARIADIC ARRAY[properties, '"val"'::agtype]));
            ANALYZE;
        """, tuples_only=False)

    def one_hop(self, start_local):
        return (f"SELECT * FROM cypher('benchg', $$ MATCH (a:P)-[:K]->(b:P) "
                f"WHERE a.val = {start_local} RETURN count(b) $$) AS (c agtype)")

    def n_hop(self, start_local, hops):
        return (f"SELECT * FROM cypher('benchg', $$ MATCH (a:P)-[:K*1..{hops}]->(b:P) "
                f"WHERE a.val = {start_local} RETURN count(DISTINCT b) $$) AS (c agtype)")

    def reach_hop(self, start_local, hops):
        return (f"SELECT * FROM cypher('benchg', $$ MATCH (a:P)-[:K*1..{hops}]->(b:P) "
                f"WHERE a.val = {start_local} AND b.val <> {start_local} "
                f"RETURN count(DISTINCT b) $$) AS (c agtype)")

    def prop_scan(self):
        return ("SELECT * FROM cypher('benchg', $$ MATCH (a:P) WHERE a.val < 100 "
                "RETURN count(a) $$) AS (c agtype)")


class AGEExplicit(AGE):
    """
    Apache AGE asked the same question without `*1..n`.

    TypeQL has no variable-length path operator, so TypeDB is *forced* to spell
    the depths out. Giving AGE the same option separates two very different
    claims: "AGE's storage is slow" and "AGE's variable-length path operator is
    slow". Only one of them turns out to be true.
    """
    name = "age_explicit"
    db = "bench_age"
    reuses = "age"

    def setup(self, n_nodes, edges):
        pass  # reuses the database AGE already built

    def n_hop(self, start_local, hops):
        parts = []
        for depth in range(1, hops + 1):
            pattern = "(a:P)" + "".join(
                f"-[:K]->({'b' if i == depth - 1 else 'm' + str(i)}:P)" for i in range(depth))
            parts.append(f"SELECT * FROM cypher('benchg', $$ MATCH {pattern} "
                         f"WHERE a.val = {start_local} RETURN b $$) AS (c agtype)")
        return "SELECT count(DISTINCT c) FROM (" + " UNION ALL ".join(parts) + ") t"

    def reach_hop(self, start_local, hops):
        parts = []
        for depth in range(1, hops + 1):
            pattern = "(a:P)" + "".join(
                f"-[:K]->({'b' if i == depth - 1 else 'm' + str(i)}:P)" for i in range(depth))
            parts.append(f"SELECT * FROM cypher('benchg', $$ MATCH {pattern} "
                         f"WHERE a.val = {start_local} AND b.val <> {start_local} "
                         f"RETURN b $$) AS (c agtype)")
        return "SELECT count(DISTINCT c) FROM (" + " UNION ALL ".join(parts) + ") t"


class CTE(PgSystem):
    """Plain PostgreSQL: two tables and a recursive CTE. The floor to beat."""
    name = "cte"
    db = "bench_cte"
    prelude = ""

    def setup(self, n_nodes, edges):
        recreate(self.db)
        psql(self.db, f"""
            CREATE TABLE n (id int PRIMARY KEY, name text, val int);
            CREATE TABLE e (id serial PRIMARY KEY, src int, dst int);
            INSERT INTO n SELECT i, 'n'||i, i FROM generate_series(0,{n_nodes-1}) i;
        """.replace("%%", "%"), tuples_only=False)
        values = ",".join(f"({s},{d})" for s, d in edges)
        psql(self.db, f"""
            INSERT INTO e (src,dst) VALUES {values};
            CREATE INDEX ON e(src); CREATE INDEX ON e(dst); CREATE INDEX ON n(val);
            ANALYZE;
        """, tuples_only=False)

    def one_hop(self, start_local):
        return f"SELECT count(*) FROM e WHERE src = {start_local}"

    def n_hop(self, start_local, hops):
        return f"""
            WITH RECURSIVE w(node, depth, path) AS (
                SELECT {start_local}, 0, ARRAY[]::int[]
              UNION ALL
                SELECT e.dst, w.depth+1, w.path || e.id
                  FROM w JOIN e ON e.src = w.node
                 WHERE w.depth < {hops} AND NOT (e.id = ANY(w.path)))
            SELECT count(DISTINCT node) FROM w WHERE depth > 0"""

    def reach_hop(self, start_local, hops):
        # The plain-SQL floor gets the same freedom every other system has:
        # asked for reachability, it may answer with a visited set instead of
        # enumerating trails. `UNION` deduplicates the worktable, which is what
        # a hand-written CTE would do once the author noticed the difference.
        return f"""
            WITH RECURSIVE w(node, depth) AS (
                SELECT {start_local}, 0
              UNION
                SELECT e.dst, w.depth+1
                  FROM w JOIN e ON e.src = w.node
                 WHERE w.depth < {hops})
            SELECT count(DISTINCT node) FROM w
             WHERE depth > 0 AND node <> {start_local}"""

    def prop_scan(self):
        return "SELECT count(*) FROM n WHERE val < 100"


class PgGraph(PgSystem):
    """
    pgGraph 1.1 — a graph *index* over ordinary tables, not a graph store.

    It reads the same two plain tables the `cte` system uses, because that is
    what it is designed for: your tables stay the source of truth and
    `graph.build()` compiles their topology into a backend-local CSR artifact.
    So this row measures the architecture rather than a data model.

    Three things about the comparison have to be stated rather than assumed:

    - **It answers reachability, not paths.** `graph.traverse()` has no
      variable-length pattern and no trail semantics; `uniqueness :=
      'node_global'` reports each reached node once. That is exactly the
      question `count(DISTINCT b)` asks, which is why this comparison is
      meaningful at all — and it is why there is no pgGraph column for a query
      that binds a path.
    - **`hydrate := false`.** Asked to hydrate, it fetches the source rows,
      which is a different amount of work from counting ids. The Cypher systems
      are counting node identity, so pgGraph is asked for identity too.
    - **`max_rows` must be raised.** It defaults to 1000, and a capped result is
      a wrong answer, not a fast one. The correctness gate catches this if it is
      ever missed.

    `graph.build()` is run during setup and is *not* in any timing, matching how
    the CSR compile is reported separately in `docs/deep-traversal.md`. The
    per-backend load cost that follows a build is charged to whoever measures a
    cold connection; this harness measures warm ones, for everybody.
    """
    name = "pggraph"
    db = "bench_pggraph"
    # pgGraph caps each statement at 64 MB of working memory by default and
    # refuses the query rather than swapping — the circuit breaker doing its
    # job. A full traversal of a 50,000-node graph does not fit in that, so it
    # is raised here for the same reason AGE is given the indexes its
    # documentation asks for: benchmarking a default that its own docs tell you
    # to change measures the default, not the engine.
    prelude = "SET graph.query_memory_mb = 4096; SET graph.query_work_limit = 200000000"

    def version(self):
        try:
            return "pgGraph " + psql(
                "postgres", "SELECT default_version FROM pg_available_extensions "
                            "WHERE name='graph'")
        except Exception:
            return "pgGraph"

    def available(self):
        try:
            return psql("postgres", "SELECT count(*) FROM pg_available_extensions "
                                    "WHERE name='graph'") == "1"
        except Exception:
            return False

    def setup(self, n_nodes, edges):
        self.n_nodes = n_nodes
        recreate(self.db)
        psql(self.db, "CREATE EXTENSION graph", tuples_only=False)
        psql(self.db, f"""
            CREATE TABLE n (id int PRIMARY KEY, name text, val int);
            CREATE TABLE e (id serial PRIMARY KEY, src int, dst int);
            INSERT INTO n SELECT i, 'n'||i, i FROM generate_series(0,{n_nodes-1}) i;
        """.replace("%%", "%"), tuples_only=False)
        values = ",".join(f"({s},{d})" for s, d in edges)
        psql(self.db, f"""
            INSERT INTO e (src,dst) VALUES {values};
            CREATE INDEX ON e(src); CREATE INDEX ON e(dst); CREATE INDEX ON n(val);
            ANALYZE;
        """, tuples_only=False)
        # Register the two tables and compile. `e` is not registered as a node
        # table, so pgGraph reads both endpoints from it — its edge-table mode.
        psql(self.db, """
            SELECT graph.add_table('public.n'::regclass, id_column := 'id',
                                   columns := ARRAY['val']);
            SELECT graph.add_edge(from_table := 'public.e'::regclass,
                                  from_column := 'src',
                                  to_table := 'public.n'::regclass,
                                  to_column := 'dst',
                                  label := 'k', bidirectional := false);
        """, tuples_only=False)
        psql(self.db, "SELECT * FROM graph.build()", tuples_only=False, timeout=1800)

    # Circuit breakers, sized to the graph rather than switched off. pgGraph
    # pre-allocates in proportion to these, so "set them to infinity" is
    # rejected outright — which is the safety property working. They are set
    # just above what the whole graph can produce, so no answer is ever capped
    # and no memory is reserved for a frontier that cannot exist.
    def _limits(self):
        n = getattr(self, "n_nodes", 100000)
        return n + 1000

    def _traverse(self, start_local, hops):
        cap = self._limits()
        return (f"SELECT count(*) FROM graph.traverse('public.n'::regclass, "
                f"'{start_local}', max_depth := {hops}, direction := 'out', "
                f"uniqueness := 'node_global', include_start := false, "
                f"hydrate := false, max_rows := {cap}, "
                f"max_nodes := {cap}, max_frontier := {cap})")

    def one_hop(self, start_local):
        return self._traverse(start_local, 1)

    def n_hop(self, start_local, hops):
        return self._traverse(start_local, hops)

    # `include_start := false` is exactly "every node but the start", so the
    # normalised question needs no adjustment on this side.
    reach_hop = n_hop

    def prop_scan(self):
        # No graph question — the source table answers it, which is the point of
        # a derived index that does not own the data.
        return "SELECT count(*) FROM n WHERE val < 100"


# --------------------------------------------------------------------------
# external servers — Neo4j and TypeDB
#
# These do not live in the benchmark's PostgreSQL, so page counts are not
# comparable and latency is measured client-side on a reused connection. That
# is the same thing psql's \timing measures, and the loopback round-trip
# (~0.1-0.3 ms) is charged to every system, including the PostgreSQL ones.
# --------------------------------------------------------------------------

def client_timed(call, queries, runs, warmup=None):
    """
    Wall-clock per query on an already-open connection, results consumed.

    The warm-up is much longer than the psql path needs, because these clients
    are Python and take tens of calls to reach steady state: measured on an
    empty query, bolt reports 2.05 ms after 2 warm-up calls and 0.73 ms after
    50. Charging Neo4j for that ramp would be measuring the driver's start-up,
    not the database.
    """
    warmup = max(50, len(queries)) if warmup is None else warmup
    times = []
    for i in range(warmup + runs):
        q = queries[i % len(queries)]
        t0 = time.perf_counter()
        call(q)
        times.append((time.perf_counter() - t0) * 1000.0)
    times = sorted(times[warmup:])
    return {
        "median_ms": round(statistics.median(times), 3),
        "p95_ms": round(times[min(len(times) - 1, int(len(times) * 0.95))], 3),
        "min_ms": round(times[0], 3),
        "runs": len(times),
    }


class Neo4j:
    """
    Neo4j 5 over bolt. Native property graph, native index on :P(val), the
    same logical queries in the same Cypher the other Cypher systems run.
    """
    name = "neo4j"
    floor_query = "RETURN 1 AS c"
    uri = os.environ.get("NEO4J_URI", "bolt://localhost:27687")
    user = os.environ.get("NEO4J_USER", "neo4j")
    password = os.environ.get("NEO4J_PASSWORD", "benchpass123")

    def __init__(self):
        self.driver = None
        self.session = None
        self._version = "Neo4j"

    def available(self):
        try:
            import neo4j  # noqa: F401
        except ImportError:
            return False
        try:
            self._connect()
            return True
        except Exception as e:
            print(f"  ({self.name}: {str(e)[:120]})")
            return False

    def _connect(self):
        import neo4j
        if self.driver is None:
            self.driver = neo4j.GraphDatabase.driver(
                self.uri, auth=(self.user, self.password))
            self.driver.verify_connectivity()
            rec = self.driver.execute_query(
                "CALL dbms.components() YIELD name, versions, edition "
                "RETURN name, versions[0] AS v, edition").records[0]
            self._version = f"Neo4j {rec['v']} ({rec['edition']})"

    def version(self):
        return self._version

    def setup(self, n_nodes, edges):
        self._connect()
        with self.driver.session() as s:
            s.run("MATCH (n) CALL { WITH n DETACH DELETE n } IN TRANSACTIONS OF 20000 ROWS")
            s.run("CREATE INDEX p_val IF NOT EXISTS FOR (p:P) ON (p.val)")
            s.run("CALL db.awaitIndexes(300)")
            for i in range(0, n_nodes, 10000):
                s.run("UNWIND range($lo, $hi) AS i CREATE (:P {name: 'n' + toString(i), val: i})",
                      lo=i, hi=min(i + 10000, n_nodes) - 1)
            for i in range(0, len(edges), 10000):
                s.run("UNWIND $rows AS r MATCH (a:P {val: r[0]}), (b:P {val: r[1]}) "
                      "CREATE (a)-[:K]->(b)",
                      rows=[list(e) for e in edges[i:i + 10000]])
            s.run("CALL db.awaitIndexes(300)")
            got = s.run("MATCH ()-[r:K]->() RETURN count(r) AS c").single()["c"]
            if got != len(edges):
                raise RuntimeError(f"loaded {got} edges, expected {len(edges)}")
        self.session = self.driver.session()

    def one_hop(self, start_local):
        return f"MATCH (a:P {{val: {start_local}}})-[:K]->(b:P) RETURN count(b) AS c"

    def n_hop(self, start_local, hops):
        return (f"MATCH (a:P {{val: {start_local}}})-[:K*1..{hops}]->(b:P) "
                f"RETURN count(DISTINCT b) AS c")

    def reach_hop(self, start_local, hops):
        return (f"MATCH (a:P {{val: {start_local}}})-[:K*1..{hops}]->(b:P) "
                f"WHERE b.val <> {start_local} RETURN count(DISTINCT b) AS c")

    def prop_scan(self):
        return "MATCH (a:P) WHERE a.val < 100 RETURN count(a) AS c"

    def _run(self, q):
        return [r["c"] for r in self.session.run(q)]

    def answer(self, query, timeout_s=None):
        return str(self._run(query)[0])

    def measure(self, queries, runs, warmup=None, timeout_s=None):
        return client_timed(self._run, queries, runs, warmup)

    def teardown(self):
        if self.session:
            self.session.close()
        if self.driver:
            self.driver.close()


class TypeDB:
    """
    TypeDB 3 over its native driver. Edges are relations (the only way TypeDB
    models them), `val` is a @key — TypeDB's own index — and variable-length
    traversal is written as an explicit disjunction of depths, because TypeQL
    has no `*1..n` operator.
    """
    name = "typedb"
    # TypeQL has no constant-only query, so the floor here is a single @key
    # lookup — an upper bound on the protocol cost, not a pure round trip.
    floor_query = "match $a isa P, has val 0; reduce $c = count;"
    addr = os.environ.get("TYPEDB_ADDR", "localhost:21729")
    user = os.environ.get("TYPEDB_USER", "admin")
    password = os.environ.get("TYPEDB_PASSWORD", "password")
    db = "benchg"
    CHUNK = int(os.environ.get("TYPEDB_CHUNK", "5000"))     # relations per write txn
    WORKERS = int(os.environ.get("TYPEDB_WORKERS", "8"))    # concurrent write txns

    SCHEMA = """
    define
      attribute name, value string;
      attribute val, value integer;
      entity P, owns name, owns val @key, plays K:src, plays K:dst;
      relation K, relates src, relates dst;
    """

    def __init__(self):
        self.driver = None
        self.tx = None
        self._version = "TypeDB"

    def available(self):
        try:
            from typedb.driver import TypeDB as TDB, Credentials, DriverOptions
            from typedb.api.connection.driver_tls_config import DriverTlsConfig
        except ImportError:
            return False
        try:
            self.driver = TDB.driver(self.addr, Credentials(self.user, self.password),
                                     DriverOptions(DriverTlsConfig.disabled()))
            self._version = f"TypeDB {self._server_version()}"
            return True
        except Exception as e:
            print(f"  ({self.name}: {str(e)[:120]})")
            return False

    def _server_version(self):
        try:
            import subprocess as sp
            out = sp.run(["docker", "exec", os.environ.get("TYPEDB_CONTAINER", "bench-typedb"),
                          "./typedb", "server", "--version"],
                         capture_output=True, text=True, timeout=60).stdout
            return out.strip().split()[-1]
        except Exception:
            return "3.x"

    def version(self):
        return self._version

    def setup(self, n_nodes, edges):
        import threading
        from typedb.driver import TransactionType

        if self.driver.databases.contains(self.db):
            self.driver.databases.get(self.db).delete()
        self.driver.databases.create(self.db)
        with self.driver.transaction(self.db, TransactionType.SCHEMA) as tx:
            tx.query(self.SCHEMA).resolve()
            tx.commit()

        with self.driver.transaction(self.db, TransactionType.WRITE) as tx:
            for i in range(0, n_nodes, 500):
                tx.query("insert " + " ".join(
                    f'$n{j} isa P, has name "n{j}", has val {j};'
                    for j in range(i, min(i + 500, n_nodes)))).resolve()
            tx.commit()

        # One relation per query is TypeDB's fastest path here: batching pairs
        # into a single match makes the pattern quadratically harder to plan
        # (measured: 500 edges/s at batch 1, 240 at batch 5). Throughput comes
        # from concurrency instead — several write transactions at once, each
        # kept small. A single transaction spanning ~10^5 relations is what
        # made a first attempt at a million edges die with TSV13
        # ("execution interrupted by a concurrent transaction close"), so the
        # work is chunked and each chunk retried once on its own.
        edge_q = ("match $a isa P, has val {s}; $b isa P, has val {d}; "
                  "insert (src: $a, dst: $b) isa K;")
        chunks = [edges[i:i + self.CHUNK] for i in range(0, len(edges), self.CHUNK)]
        nxt = itertools.count()
        lock = threading.Lock()
        errors = []

        def write_chunk(part):
            with self.driver.transaction(self.db, TransactionType.WRITE) as tx:
                pending = []
                for s, d in part:
                    pending.append(tx.query(edge_q.format(s=s, d=d)))
                    if len(pending) >= 64:
                        for p in pending:
                            p.resolve()
                        pending = []
                for p in pending:
                    p.resolve()
                tx.commit()

        def worker():
            while True:
                i = next(nxt)
                if i >= len(chunks) or errors:
                    return
                for attempt in (1, 2):
                    try:
                        write_chunk(chunks[i])
                        break
                    except Exception as e:
                        if attempt == 2:
                            with lock:
                                errors.append(f"chunk {i}: {type(e).__name__}: {str(e)[:300]}")

        threads = [threading.Thread(target=worker) for _ in range(self.WORKERS)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        if errors:
            raise RuntimeError("; ".join(errors[:3]))

        self.tx = self.driver.transaction(self.db, TransactionType.READ)
        loaded = self._run("match $r isa K; reduce $c = count;")[0]
        if loaded != len(edges):
            raise RuntimeError(f"loaded {loaded} edges, expected {len(edges)}")

    def one_hop(self, start_local):
        return (f"match $a isa P, has val {start_local}; "
                f"$r isa K (src: $a, dst: $b); reduce $c = count;")

    def n_hop(self, start_local, hops):
        # No `*1..n` in TypeQL: the depths are spelled out as a disjunction.
        branches = []
        for depth in range(1, hops + 1):
            chain, prev = [], "$a"
            for step in range(depth):
                nxt = "$b" if step == depth - 1 else f"$m{depth}_{step}"
                chain.append(f"$r{depth}_{step} isa K (src: {prev}, dst: {nxt});")
                prev = nxt
            branches.append("{ " + " ".join(chain) + " }")
        return (f"match $a isa P, has val {start_local}; "
                + " or ".join(branches) + "; select $b; distinct; reduce $c = count;")

    def prop_scan(self):
        return "match $a isa P, has val < 100; reduce $c = count;"

    def _run(self, q):
        return [row.get("c").as_value().get() for row in self.tx.query(q).resolve()]

    def answer(self, query, timeout_s=None):
        return str(self._run(query)[0])

    def measure(self, queries, runs, warmup=None, timeout_s=None):
        return client_timed(self._run, queries, runs, warmup)

    def teardown(self):
        if self.tx:
            self.tx.close()
        if self.driver:
            self.driver.close()


SYSTEMS = {c.name: c for c in
           (Ontological, OntologicalRaw, AGE, AGEExplicit, CTE, PgGraph, Neo4j, TypeDB)}


# --------------------------------------------------------------------------
# workload
# --------------------------------------------------------------------------

def run(args):
    n_nodes = args.scale
    if args.shape == "grid":
        # A lattice has to be square, so the node count is rounded to fit.
        n_nodes = int(round(n_nodes ** 0.5)) ** 2
    edges = gen_edges(n_nodes, args.degree, shape=args.shape)
    wanted = [s.strip() for s in args.systems.split(",") if s.strip()]

    report = {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "scale": {"nodes": n_nodes, "edges": len(edges),
                  "avg_degree": round(len(edges) / n_nodes, 2), "shape": args.shape},
        "environment": {
            "postgres": psql("postgres", "SELECT version()"),
            "host": f"{PGHOST}:{PGPORT}",
        },
        "systems": {},
        "correctness": {},
    }

    live = {}
    for name in wanted:
        if name not in SYSTEMS:
            print(f"! unknown system '{name}'", file=sys.stderr)
            continue
        sysobj = SYSTEMS[name]()
        if hasattr(sysobj, "available") and not sysobj.available():
            print(f"~ skipping {name}: extension not installed")
            report["systems"][name] = {"skipped": "extension not installed"}
            continue
        print(f"→ loading {name} ({n_nodes} nodes, {len(edges)} edges)")
        t0 = time.perf_counter()
        try:
            sysobj.setup(n_nodes, edges)
        except Exception as e:
            print(f"! {name} setup failed: {e}")
            report["systems"][name] = {"error": str(e)[:2000]}
            continue
        load_s = time.perf_counter() - t0
        live[name] = sysobj
        # A system that reads another one's database did not load anything;
        # reporting `len(edges) / ~0 seconds` would publish a made-up number.
        reuses = getattr(sysobj, "reuses", None)
        report["systems"][name] = {
            "engine": sysobj.version() if hasattr(sysobj, "version") else None,
            "reuses": reuses,
            "load_seconds": None if reuses else round(load_s, 2),
            "load_edges_per_sec": None if reuses else round(len(edges) / load_s),
            "queries": {},
        }

    if not live:
        print("no systems available")
        return report

    starts = [7, 42, 101, 512, 900]

    # --- correctness gate: every system must agree before timings mean anything
    print("→ correctness check")
    hops_wanted = [int(h) for h in str(args.hops).split(",") if h.strip()]
    q = "reach_hop" if args.workload == "reach" else "n_hop"
    checks = ([("1hop", lambda s, st: s.one_hop(st))] if args.workload != "reach" else []) + \
             [(f"{'reach' if args.workload == 'reach' else ''}{h}hop",
               (lambda h: lambda s, st: getattr(s, q)(st, h))(h))
              for h in hops_wanted] + \
             [("prop_scan", lambda s, _st: s.prop_scan())]
    dead = {}
    for label, maker in checks:
        answers = {}
        for name, s in live.items():
            if name in dead:
                answers[name] = f"error: {dead[name]}"
                continue
            try:
                answers[name] = s.answer(maker(s, starts[0]), args.query_timeout)
            except Crashed as e:
                # One system's crash restarts the postmaster and would void
                # everyone else's numbers. Record it, stop asking this system
                # anything deeper, and wait for the server before moving on.
                dead[name] = f"crashed the server at {label}"
                print(f"  ! {name} crashed the server at {label} — not asked anything deeper")
                answers[name] = f"error: crashed ({str(e)[:80]})"
                wait_for_server()
            except Exception as e:
                # A system that cannot finish inside the cap has no answer to
                # disagree with; that is a missing cell, not a wrong one.
                answers[name] = f"error: {e}"[:200]
                if isinstance(e, Timeout):
                    dead[name] = f"exceeded {args.query_timeout}s at {label}"
        distinct = {v for v in answers.values() if not v.startswith("error")}
        report["correctness"][label] = {
            "answers": answers,
            "agree": len(distinct) <= 1,
        }
        if len(distinct) > 1:
            print(f"  ! {label}: systems disagree {answers} — timings for this query are VOID")

    # --- protocol floor: what a trivial query costs on each client path.
    # Published next to the timings because bolt and psql do not charge the
    # same round trip, and a millisecond of driver overhead is a large part of
    # a one-hop answer.
    print("→ protocol floor")
    for name, s in live.items():
        try:
            m = s.measure([s.floor_query], max(4, args.runs))
            report["systems"][name]["protocol_floor_ms"] = m["median_ms"]
            print(f"    {name:<12} {m['median_ms']:>9.3f} ms")
        except Exception as e:
            print(f"    {name:<12} error: {str(e)[:100]}")

    # --- timings
    #
    # Deep hops need two things the shallow workload did not. A per-statement
    # cap, because a system that enumerates paths does not return at six hops
    # and the run has to finish. And a give-up rule: once a system has blown
    # the cap at depth k it is not asked depth k+1, because the answer is known
    # and the wall-clock is not free. Both are recorded in the results file, so
    # a blank cell says which of the two produced it.
    hops = [int(h) for h in str(args.hops).split(",") if h.strip()]
    reach = args.workload == "reach"
    q = "reach_hop" if reach else "n_hop"
    workload = ([] if reach else [("1hop", lambda s, st: s.one_hop(st))]) + \
               [(f"{'reach' if reach else ''}{h}hop",
                 (lambda h: lambda s, st: getattr(s, q)(st, h))(h))
                for h in hops if reach or h > 1] + \
               [("prop_scan", lambda s, _st: s.prop_scan())]
    gave_up = dict(dead)

    for label, maker in workload:
        print(f"→ {label}")
        digits = "".join(c for c in label if c.isdigit())
        deep = digits != "" and int(digits) >= 4
        for name, s in live.items():
            if name in gave_up:
                report["systems"][name]["queries"][label] = {
                    "not_attempted": f"exceeded {args.query_timeout}s at {gave_up[name]}"}
                print(f"    {name:<12} not attempted ({gave_up[name]} already exceeded the cap)")
                continue
            sqls = [maker(s, st) for st in starts]
            try:
                # A fifty-call warm-up is right for a bolt driver on a
                # sub-millisecond query and absurd on one that takes a second.
                m = s.measure(sqls, args.runs if not deep else min(args.runs, 3),
                              warmup=1 if deep else None,
                              timeout_s=args.query_timeout)
                report["systems"][name]["queries"][label] = m
                print(f"    {name:<12} {m['median_ms']:>9.3f} ms"
                      + (f"   {m['buffers']:>7} pages" if "buffers" in m else ""))
            except Timeout as t:
                report["systems"][name]["queries"][label] = {"timeout_s": t.seconds}
                gave_up[name] = label
                print(f"    {name:<12} > {t.seconds}s — did not finish")
            except Crashed as c:
                report["systems"][name]["queries"][label] = {"crashed": str(c)[:200]}
                gave_up[name] = label
                print(f"    {name:<12} crashed the server — not asked anything deeper")
                wait_for_server()
            except Exception as e:
                report["systems"][name]["queries"][label] = {"error": str(e)[:400]}
                print(f"    {name:<12} error: {str(e)[:120]}")

    # --- speedups relative to each other
    if "ontological" in report["systems"]:
        base = report["systems"]["ontological"].get("queries", {})
        for other in ("age", "cte", "ontological_raw", "neo4j", "typedb"):
            o = report["systems"].get(other, {}).get("queries")
            if not o:
                continue
            sp = {}
            for q, m in base.items():
                if "median_ms" in m and q in o and "median_ms" in o[q] and m["median_ms"] > 0:
                    sp[q] = round(o[q]["median_ms"] / m["median_ms"], 2)
            report.setdefault("speedup_vs", {})[other] = sp

    # --- structural integrity is part of the result, not a separate concern
    if "ontological" in live:
        bad = psql(live["ontological"].db, "SELECT count(*) FROM og_check_integrity()")
        report["integrity_violations"] = int(bad)

    for s in live.values():
        try:
            s.teardown()
        except Exception:
            pass

    RESULTS.mkdir(exist_ok=True)
    out = RESULTS / f"bench-{n_nodes}-{datetime.now(timezone.utc):%Y%m%dT%H%M%SZ}.json"
    out.write_text(json.dumps(report, indent=2))
    print(f"\nwritten: {out}")
    print_summary(report)
    return report


def print_summary(r):
    print("\n" + "=" * 74)
    print(f"  {r['scale']['nodes']} nodes / {r['scale']['edges']} edges")
    print("=" * 74)
    names = [n for n, v in r["systems"].items() if "queries" in v]
    if not names:
        return
    queries = sorted({q for n in names for q in r["systems"][n]["queries"]})
    print(f"{'query':<12}" + "".join(f"{n:>16}" for n in names))
    for q in queries:
        row = f"{q:<12}"
        for n in names:
            m = r["systems"][n]["queries"].get(q, {})
            row += f"{m.get('median_ms', float('nan')):>13.2f} ms" if "median_ms" in m else f"{'—':>16}"
        print(row)
    for other, sp in r.get("speedup_vs", {}).items():
        if sp:
            print(f"\nontological vs {other}: " +
                  ", ".join(f"{q} {v}×" for q, v in sp.items()))
    bad = [k for k, v in r["correctness"].items() if not v["agree"]]
    if bad:
        print(f"\n!! systems disagreed on {bad} — those timings are void")
    else:
        print("\nall systems returned identical answers")
    if "integrity_violations" in r:
        print(f"structural integrity violations: {r['integrity_violations']}")


def compare(baseline_path, current):
    """CI regression gate — spec 009 FR-019..FR-023."""
    base = json.loads(Path(baseline_path).read_text())
    threshold = 1.20
    failures = []
    for name, cur_sys in current["systems"].items():
        for q, m in cur_sys.get("queries", {}).items():
            b = base.get("systems", {}).get(name, {}).get("queries", {}).get(q, {})
            if "median_ms" not in m or "median_ms" not in b or b["median_ms"] <= 0:
                continue
            ratio = m["median_ms"] / b["median_ms"]
            if ratio > threshold:
                failures.append(f"{name}/{q}: {b['median_ms']:.2f} → {m['median_ms']:.2f} ms "
                                f"({ratio:.2f}× slower)")
    if failures:
        print("\nREGRESSION detected:")
        for f in failures:
            print("  " + f)
        return 1
    print("\nno regression against baseline")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--scale", type=int, default=20000, help="node count")
    ap.add_argument("--degree", type=int, default=8, help="average out-degree")
    ap.add_argument("--runs", type=int, default=10)
    ap.add_argument(
        "--systems",
        default="ontological,ontological_raw,age,age_explicit,cte,neo4j,typedb")
    ap.add_argument("--hops", default="2,3",
                    help="traversal depths to measure, e.g. 2,3,4,5,6,8")
    ap.add_argument("--shape", choices=("random", "chain", "grid"), default="random",
                    help="graph shape. random is the published workload; chain "
                         "and grid have a large diameter, which is the only way "
                         "a question about depth means anything")
    ap.add_argument("--workload", choices=("classic", "reach"), default="classic",
                    help="classic: the published 1/2/3-hop workload, one row per "
                         "path. reach: distinct nodes other than the start, the "
                         "only question every system states identically")
    ap.add_argument("--query-timeout", type=int, default=120,
                    help="per-statement cap in seconds; a system that blows it "
                         "is recorded as such and not asked anything deeper")
    ap.add_argument("--compare-baseline", default=None)
    args = ap.parse_args()

    report = run(args)
    if args.compare_baseline:
        sys.exit(compare(args.compare_baseline, report))


if __name__ == "__main__":
    main()
