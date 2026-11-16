#!/usr/bin/env python3
"""Capture the generated-SQL mold of the running engine into a folder.

    python3 tests/seam/capture.py --port N --out DIR [--work DIR] [--only corpus,sql,typeql,tutorials]

Called by tests/seam/capture.sh, which builds and installs the engine and the statement tracer
(tests/seam/tracer) into a throwaway cluster first. On its own it needs that cluster: PostgreSQL
on 127.0.0.1:N with `session_preload_libraries = 'og_mold_trace'`, a superuser `og` and `dev`.
What is captured and how to read it: tests/seam/README.md.

Two layers:
  sql/    what the compiler hands back: og_cypher_check, og_cypher_columns, og_cypher_sql,
          og_typeql_sql, and what running the query reported (row count or error, og_cypher_stats)
  trace/  every statement the engine ran for it (og_mold_trace): the compiled SQL that was
          executed, the catalog lookups, the storage writes, the DDL it issued, nested to any depth

Every case runs in a fixed order on databases created empty for the run, so two captures of
the same tree are the same file for file (checked by `scripts/mold.sh capture --repeat 2`).
"""
from __future__ import annotations

import argparse
import glob
import json
import os
import re
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
COMPAT = os.path.join(ROOT, "tests", "compat")
SOURCES = ("corpus", "sql", "typeql", "tutorials")

import psycopg2  # noqa: E402
import psycopg2.extensions  # noqa: E402


# ---------------------------------------------------------------- text

def esc(s: str) -> str:
    """One line: the tracer's escaping (\\ \\n \\r \\t)."""
    return s.replace("\\", "\\\\").replace("\n", "\\n").replace("\r", "\\r").replace("\t", "\\t")


class Norm:
    """What differs between two captures of the same tree and is not the engine's doing.

    - the cluster's port, data directory and work directory (each capture picks its own);
    - the time of the run: timestamps within two days of it, and epoch milliseconds;
    - elapsed time the engine reports in a message (``… after 1503 ms``).
    """

    def __init__(self, port, paths):
        self.port = str(port)
        self.paths = sorted({p for p in paths if p}, key=len, reverse=True)
        t = time.time()
        days = {time.strftime("%Y-%m-%d", time.gmtime(t + d * 86400)) for d in (-2, -1, 0, 1, 2)}
        days |= {time.strftime("%Y-%m-%d", time.localtime(t + d * 86400)) for d in (-2, -1, 0, 1, 2)}
        self.now_re = re.compile(r"(?:%s)[T ]\d\d:\d\d(?::\d\d(?:\.\d+)?)?(?:Z|[+-]\d\d(?::?\d\d)?)?"
                                 % "|".join(sorted(days)))
        self.ms_lo, self.ms_hi = int((t - 2 * 86400) * 1000), int((t + 2 * 86400) * 1000)
        self.ms_re = re.compile(r"(?<![0-9])1[0-9]{12}(?![0-9])")
        self.port_re = re.compile(r"(?<![0-9])%s(?![0-9])" % re.escape(self.port))
        self.elapsed_re = re.compile(r"(?<![0-9])[0-9]+(?:\.[0-9]+)? ?ms\b")

    def __call__(self, s: str) -> str:
        for p in self.paths:
            s = s.replace(p, "<work>")
        s = self.port_re.sub("<pgport>", s)
        s = self.now_re.sub("<now>", s)
        s = self.ms_re.sub(lambda m: "<now:ms>" if self.ms_lo <= int(m.group(0)) <= self.ms_hi else m.group(0), s)
        return s


class Trace:
    """The tracer's file for one connection or one run of a suite."""

    def __init__(self, path):
        self.path = path
        open(path, "wb").close()

    def mark(self):
        return os.path.getsize(self.path)

    def take(self, pos):
        with open(self.path, "rb") as f:
            f.seek(pos)
            data = f.read()
        return data.decode("utf-8", "backslashreplace").splitlines()

    def all(self):
        return self.take(0)


def collapse(lines):
    """Consecutive identical lines as one, with the count — a per-row function call is one line."""
    out, prev, n = [], None, 0
    for ln in lines:
        if ln == prev:
            n += 1
            continue
        if prev is not None:
            out.append(prev if n == 1 else f"{prev}  ×{n}")
        prev, n = ln, 1
    if prev is not None:
        out.append(prev if n == 1 else f"{prev}  ×{n}")
    return out


def write_lines(path, lines):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        for ln in lines:
            f.write(ln + "\n")


def err_text(e: psycopg2.Error) -> str:
    msg = (e.diag.message_primary or str(e)).strip()
    return f"{e.pgcode or '?????'} {msg}"


# ---------------------------------------------------------------- the cluster

class Cluster:
    def __init__(self, args):
        self.args = args

    def psql(self, sql=None, db="postgres", file=None, variables=None, env=None, check=True, quiet=True):
        cmd = [self.args.psql, "-X", "-h", self.args.host, "-p", str(self.args.port), "-U", self.args.user, "-d", db]
        if quiet:
            cmd.append("-q")
        for k, v in (variables or {}).items():
            cmd += ["-v", f"{k}={v}"]
        cmd += ["-f", file] if file else ["-c", sql]
        r = subprocess.run(cmd, capture_output=True, text=True, env=env or self.env())
        if check and r.returncode != 0:
            sys.exit(f"psql failed ({file or sql}):\n{r.stderr}")
        return r

    def env(self, trace=None, user=None):
        e = {k: v for k, v in os.environ.items() if k != "PGOPTIONS"}
        e.update(PGHOST=self.args.host, PGPORT=str(self.args.port), PGUSER=user or self.args.user)
        if trace:
            e["PGOPTIONS"] = f"-c og_mold_trace.file={trace}"
        return e

    def fresh(self, db):
        self.psql(f"DROP DATABASE IF EXISTS {db} WITH (FORCE)")
        self.psql(f"CREATE DATABASE {db}")

    def connect(self, db, trace=None):
        opts = f"-c og_mold_trace.file={trace}" if trace else ""
        c = psycopg2.connect(host=self.args.host, port=self.args.port, user=self.args.user, dbname=db, options=opts)
        return c


# ---------------------------------------------------------------- source 1: the compat corpus

class CorpusRun:
    """tests/compat's corpus, in run.py's order, through SQL on one connection.

    For every statement — fixture, read, scenario step — the calls the Bolt gateway makes
    (og_cypher_check, og_cypher_columns, og_cypher, og_cypher_stats), then og_cypher_sql.
    Scenarios run in one transaction each and are rolled back; a failing step is undone with a
    savepoint (run.py rolls back and replays the steps before it: the same state, fewer
    statements).
    """

    TIMEOUT_MS = 30000

    def __init__(self, cl: Cluster, out, work, norm):
        self.cl, self.out, self.work, self.norm = cl, out, work, norm
        sys.path.insert(0, COMPAT)
        sys.path.insert(0, os.path.join(COMPAT, "extract"))
        import run as compat  # noqa: E402 - tests/compat/run.py: corpus loading, params, fixtures
        self.compat = compat

        class Resolver(compat.Runner):
            def __init__(self, env):  # noqa: D401 - no Bolt driver: only resolve() is used
                self.env = env
        self.Resolver = Resolver

    def call(self, cur, sql, args, in_tx):
        if in_tx:
            cur.execute("SAVEPOINT og_mold")
        try:
            cur.execute(sql, args)
            rows = cur.fetchall()
            if in_tx:
                cur.execute("RELEASE SAVEPOINT og_mold")
            return rows, None
        except psycopg2.Error as e:
            if in_tx:
                cur.execute("ROLLBACK TO SAVEPOINT og_mold")
                cur.execute("RELEASE SAVEPOINT og_mold")
            return None, err_text(e)

    def cypher_rows(self, cur, graph, text, in_tx):
        rows, err = self.call(cur, "SELECT og_cypher(%s, %s, '{}'::jsonb)", (graph, text), in_tx)
        if err:
            return [], []
        recs = [r[0] for r in rows]
        return (list(recs[0].keys()) if recs else []), recs

    def case(self, cur, tr, sql_out, trace_out, head, graph, text, params, in_tx):
        pos = tr.mark()
        lines = [head]
        rows, err = self.call(cur, "SELECT og_cypher_check(%s)::text", (text,), in_tx)
        lines.append("CHECK " + (err or rows[0][0]))
        rows, err = self.call(cur, "SELECT og_cypher_columns(%s)", (text,), in_tx)
        lines.append("COLS " + (err or json.dumps(rows[0][0], ensure_ascii=False)))
        rows, err = self.call(cur, "SELECT og_cypher(%s, %s, %s::jsonb)::text", (graph, text, json.dumps(params)),
                              in_tx)
        lines.append("RUN " + (err or f"rows={len(rows)}"))
        ran_ok = err is None
        rows, err = self.call(cur, "SELECT og_cypher_stats()::text", (), in_tx)
        lines.append("STATS " + (err or rows[0][0]))
        rows, err = self.call(cur, "SELECT og_cypher_sql(%s, %s)", (graph, text), in_tx)
        lines.append("SQL " + (err and "ERR " + err or esc(rows[0][0])))
        sql_out += [self.norm(ln) for ln in lines]
        trace_out.append(head)
        trace_out += collapse(self.norm(ln) for ln in tr.take(pos))
        return ran_ok

    def run(self):
        compat = self.compat
        clients = ["studio", "robo", "analyzer"]
        corpus = compat.load_corpus(clients)
        graphs = sorted({g for _, man in corpus.values() for g in man["graphs"]})
        db = "og_compat"
        self.cl.fresh(db)
        self.cl.psql(db=db, file=os.path.join(COMPAT, "targets", "v1", "bootstrap.sql"),
                     variables={"graphs": ",".join(graphs)})
        trpath = os.path.join(self.work, "corpus.trace")
        tr = Trace(trpath)
        conn = self.cl.connect(db, trace=trpath)
        conn.autocommit = True
        cur = conn.cursor()
        cur.execute("SELECT set_config('og.statement_timeout', %s, false)", (str(self.TIMEOUT_MS),))
        resolver = self.Resolver({"COMPAT_PGHOST": self.cl.args.host, "COMPAT_PGPORT": str(self.cl.args.port),
                                  "COMPAT_PGDATABASE": db, "COMPAT_PGUSER": self.cl.args.user})
        subst = {"PGHOST": self.cl.args.host, "PGPORT": str(self.cl.args.port), "PGDATABASE": db,
                 "PGUSER": self.cl.args.user}
        counts = {"fixture": 0, "read": 0, "step": 0, "scenario": 0}

        sql_out, trace_out = {c: [] for c in clients}, {c: [] for c in clients}
        conf = json.load(open(os.path.join(COMPAT, "extract", "clients.json"), encoding="utf-8"))
        files = [("_shared", os.path.join(COMPAT, "corpus", "_shared", "fixture"), graphs[0])]
        files += [(c, os.path.join(d, "fixture"), conf[c]["graph"]) for c, (d, man) in corpus.items()]
        for owner, fdir, g0 in files:
            for f in sorted(glob.glob(os.path.join(fdir, "*.cypher"))):
                graph, stmts = compat.fixture_statements(f, g0, subst)
                if graph == "*":
                    graph = graphs[0]
                rel = os.path.relpath(f, COMPAT)
                key = "studio" if owner == "_shared" else owner
                for line, stmt in stmts:
                    self.case(cur, tr, sql_out[key], trace_out[key], f"=== fixture {rel}:{line} graph={graph}",
                              graph, stmt, {}, False)
                    counts["fixture"] += 1

        for c, (d, man) in corpus.items():
            byid = {q["id"]: q for q in man["queries"]}
            for q in man["queries"]:
                if q["kind"] not in ("read", "fabric-read"):
                    continue
                g = q["graph"]
                params = resolver.resolve(q["param_spec"], lambda t, g=g: self.cypher_rows(cur, g, t, False))
                self.case(cur, tr, sql_out[c], trace_out[c], f"=== read {q['id']} {q['kind']} graph={g}",
                          g, q["text"], params or {}, False)
                counts["read"] += 1
            for sc in man["scenarios"]:
                g = sc["graph"]
                conn.autocommit = False
                head = f"=== scenario {sc['name']} graph={g}"
                sql_out[c].append(head)
                trace_out[c].append(head)
                for sid in sc["steps"]:
                    q = byid[sid]
                    params = resolver.resolve(q["param_spec"], lambda t, g=g: self.cypher_rows(cur, g, t, True))
                    self.case(cur, tr, sql_out[c], trace_out[c], f"=== step {sid} {q['kind']} graph={g}",
                              g, q["text"], params or {}, True)
                    counts["step"] += 1
                conn.rollback()
                conn.autocommit = True
                counts["scenario"] += 1
        conn.close()
        for c in clients:
            write_lines(os.path.join(self.out, "sql", "corpus", c + ".txt"), sql_out[c])
            write_lines(os.path.join(self.out, "trace", "corpus", c + ".txt"), trace_out[c])
        return counts


# ---------------------------------------------------------------- og_cypher calls written as literals

CALL_RE = re.compile(r"og_cypher\(\s*'((?:[^']|'')*)'\s*,\s*")
DOLLAR_RE = re.compile(r"\$([A-Za-z_][A-Za-z_0-9]*)?\$")


def literal_cypher_calls(text):
    """(graph, query) of every `og_cypher('<graph>', <literal>…` in SQL text, in order, once each.

    The literal may be '…' (with '' inside) or $tag$…$tag$; calls built with format() or from
    variables are not literals and are left to the trace. Calls inside DO blocks and function
    bodies are found too: their text is still in the file.
    """
    out, seen = [], set()
    for m in CALL_RE.finditer(text):
        graph = m.group(1).replace("''", "'")
        pos = m.end()
        q = None
        if text.startswith("'", pos):
            i, buf = pos + 1, []
            while i < len(text):
                if text[i] == "'":
                    if text.startswith("''", i):
                        buf.append("'")
                        i += 2
                        continue
                    q = "".join(buf)
                    break
                buf.append(text[i])
                i += 1
        else:
            d = DOLLAR_RE.match(text, pos)
            if d:
                end = text.find(d.group(0), d.end())
                if end >= 0:
                    q = text[d.end():end]
        if q is None or not re.fullmatch(r"\w+", graph):
            continue
        if (graph, q) not in seen:
            seen.add((graph, q))
            out.append((graph, q))
    return out


def compile_each(cl, db, calls, norm, head="og_cypher"):
    """og_cypher_sql of each (graph, query) on a fresh connection to `db`, as sql-layer lines."""
    lines = []
    conn = cl.connect(db)
    conn.autocommit = True
    cur = conn.cursor()
    for graph, q in calls:
        lines.append(norm(f"=== {head} graph={graph} {esc(q)}"))
        try:
            cur.execute("SELECT og_cypher_sql(%s, %s)", (graph, q))
            lines.append(norm("SQL " + esc(cur.fetchone()[0])))
        except psycopg2.Error as e:
            lines.append(norm("SQL ERR " + err_text(e)))
    conn.close()
    return lines


# ---------------------------------------------------------------- source 2: the SQL regression files

def sql_files(cl: Cluster, out, work, norm):
    """tests/common/sql + tests/v1/sql in tests/run.sh's order, each on a fresh og_test, traced whole.

    Then, on that database as the file left it, og_cypher_sql of every og_cypher call the file
    writes as a literal (the trace has each one as it ran, at its point in the file).
    """
    files = sorted(glob.glob(os.path.join(ROOT, "tests", "common", "sql", "*.sql"))
                   + glob.glob(os.path.join(ROOT, "tests", "v1", "sql", "*.sql")), key=os.path.basename)
    counts = {"files": 0, "statements": 0, "og_calls": 0, "compiled": 0}
    status = []
    for f in files:
        name = os.path.basename(f)
        cl.fresh("og_test")
        cl.psql("CREATE EXTENSION ontological CASCADE", db="og_test")
        trpath = os.path.join(work, "sql-" + name + ".trace")
        tr = Trace(trpath)
        r = subprocess.run([cl.args.psql, "-X", "-q", "-h", cl.args.host, "-p", str(cl.args.port), "-d", "og_test",
                            "-f", f], capture_output=True, text=True, env=cl.env(trace=trpath), cwd=work)
        errors = sum(1 for ln in (r.stdout + r.stderr).splitlines() if re.match(r"^(ERROR|psql.*ERROR)", ln))
        lines = collapse(norm(ln) for ln in tr.all())
        write_lines(os.path.join(out, "trace", "sql", name + ".trace"), lines)
        calls = literal_cypher_calls(open(f, encoding="utf-8").read())
        write_lines(os.path.join(out, "sql", "sql", name + ".txt"), compile_each(cl, "og_test", calls, norm))
        status.append(f"{name} exit={r.returncode} errors={errors}")
        counts["files"] += 1
        counts["compiled"] += len(calls)
        counts["statements"] += sum(1 for ln in lines if ln.startswith("0 "))
        counts["og_calls"] += sum(1 for ln in lines if ln.startswith("0 ") and "og_cypher" in ln)
    write_lines(os.path.join(out, "sql", "sql-files.txt"), status)
    return counts


# ---------------------------------------------------------------- source 3: TypeQL

def typeql_suite(cl: Cluster, out, work, norm):
    """tests/v1/typeql/run.py as it is, with og_typeql_sql asked of every query it sends."""
    trpath = os.path.join(work, "typeql.trace")
    tr = Trace(trpath)
    env_before = dict(os.environ)
    os.environ.update(cl.env(trace=trpath))
    sys.path.insert(0, os.path.join(ROOT, "tests", "v1", "typeql"))
    import importlib
    suite = importlib.import_module("run")
    if not getattr(suite, "__file__", "").startswith(os.path.join(ROOT, "tests", "v1", "typeql")):
        # tests/compat/run.py was imported first under the same name
        sys.modules.pop("run")
        suite = importlib.import_module("run")
    original = suite.psql
    asked = []
    pat = re.compile(r"SELECT og_typeql\('(\w+)', \$tql\$(.*)\$tql\$\)\Z", re.S)

    def psql(db, sql, tuples=True):
        m = pat.match(sql)
        if m:
            r = original(db, f"SELECT og_typeql_sql('{m.group(1)}', $tql${m.group(2)}$tql$)")
            got = esc(r.stdout.rstrip("\n")) if r.returncode == 0 else "ERR " + esc(r.stderr.strip())
            asked.append(f"=== typeql graph={m.group(1)} {esc(m.group(2))}")
            asked.append("SQL " + got)
        r = original(db, sql, tuples)
        if m:
            asked.append("RUN " + ("ok" if r.returncode == 0 else "ERR " + esc(r.stderr.strip())))
        return r
    suite.psql = psql
    log = os.path.join(work, "typeql.log")
    stdout = sys.stdout
    try:
        with open(log, "w") as f:
            sys.stdout = f
            sys.argv = ["run.py"]
            rc = suite.main()
    finally:
        sys.stdout = stdout
        os.environ.clear()
        os.environ.update(env_before)
        sys.modules.pop("run", None)
    write_lines(os.path.join(out, "sql", "typeql.txt"), [norm(ln) for ln in asked] + [f"# suite exit {rc}"])
    write_lines(os.path.join(out, "trace", "typeql.trace"), collapse(norm(ln) for ln in tr.all()))
    return {"queries": sum(1 for ln in asked if ln.startswith("=== "))}


# ---------------------------------------------------------------- source 4: the tutorials

def tutorials(cl: Cluster, out, work, norm, python):
    """examples/tutorials/build_tutorial.py --check --all, as tests/tutorials.sh runs it (as dev)."""
    trpath = os.path.join(work, "tutorials.trace")
    tr = Trace(trpath)
    env = cl.env(trace=trpath, user="dev")
    env.update(TUT_EXEC="env", TUT_PGPORT=str(cl.args.port))
    r = subprocess.run([python, os.path.join(ROOT, "examples", "tutorials", "build_tutorial.py"), "--check", "--all"],
                       capture_output=True, text=True, env=env, cwd=ROOT)
    with open(os.path.join(work, "tutorials.log"), "w") as f:
        f.write(r.stdout + r.stderr)
    lines = collapse(norm(ln) for ln in tr.all())
    write_lines(os.path.join(out, "trace", "tutorials.trace"), lines)
    sys.path.insert(0, os.path.join(ROOT, "examples", "tutorials"))
    import build_tutorial as bt
    sql_lines = [f"# build_tutorial.py --check --all exit {r.returncode}"]
    compiled = 0
    for src in sorted(glob.glob(os.path.join(ROOT, "examples", "tutorials", "*", "tutorial.src.md"))):
        name = os.path.basename(os.path.dirname(src))
        text = open(src, encoding="utf-8").read()
        if not bt.FENCE_ANY.search(text) or os.path.exists(os.path.join(os.path.dirname(src), "build.py")):
            continue
        db = "tut_" + re.sub(r"\W", "_", name)
        rows = text.splitlines()
        default_graph = "default"
        if rows and (m := re.match(r"<!--\s*graph:\s*(\w+)\s*-->", rows[0])):
            default_graph, rows = m.group(1), rows[1:]
        calls, i = [], 0
        while i < len(rows):
            m = bt.FENCE.match(rows[i])
            if not m:
                i += 1
                continue
            j = i + 1
            while rows[j].rstrip() != "```":
                j += 1
            body = "\n".join(rows[i + 1:j])
            if m.group(1) == "cypher":
                g = re.search(r"graph=(\w+)", m.group(3))
                calls.append((g.group(1) if g else default_graph, body))
            else:
                calls += literal_cypher_calls(body)
            i = j + 1
        sql_lines.append(f"# {name} ({db})")
        sql_lines += compile_each(cl, db, list(dict.fromkeys(calls)), norm)
        compiled += len(dict.fromkeys(calls))
    write_lines(os.path.join(out, "sql", "tutorials.txt"), sql_lines)
    return {"statements": sum(1 for ln in lines if ln.startswith("0 ")), "compiled": compiled}


# ---------------------------------------------------------------- main

def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, required=True)
    ap.add_argument("--user", default="og")
    ap.add_argument("--psql", default="psql")
    ap.add_argument("--out", required=True, help="mold folder to write (replaced)")
    ap.add_argument("--work", required=True, help="scratch folder: raw traces, logs")
    ap.add_argument("--data-dir", default="", help="the cluster's data directory (normalised away)")
    ap.add_argument("--only", default=",".join(SOURCES))
    args = ap.parse_args()
    only = [s for s in args.only.split(",") if s]
    for s in only:
        if s not in SOURCES:
            sys.exit(f"--only: unknown source {s} (one of {', '.join(SOURCES)})")
    out = os.path.abspath(args.out)
    work = os.path.abspath(args.work)
    if os.path.isdir(out):
        import shutil
        shutil.rmtree(out)
    os.makedirs(out)
    os.makedirs(work, exist_ok=True)
    norm = Norm(args.port, [work, args.data_dir, os.path.realpath(work),
                            os.path.realpath(args.data_dir) if args.data_dir else ""])
    cl = Cluster(args)
    summary, timing = {}, {}
    for s in SOURCES:
        if s not in only:
            continue
        t0 = time.time()
        if s == "corpus":
            summary[s] = CorpusRun(cl, out, work, norm).run()
        elif s == "sql":
            summary[s] = sql_files(cl, out, work, norm)
        elif s == "typeql":
            summary[s] = typeql_suite(cl, out, work, norm)
        else:
            summary[s] = tutorials(cl, out, work, norm, sys.executable)
        timing[s] = round(time.time() - t0, 1)
        print(f"  {s:<10} {json.dumps(summary[s])}  ({timing[s]}s)", flush=True)
    with open(os.path.join(out, "summary.json"), "w") as f:
        json.dump(summary, f, indent=1, sort_keys=True)
        f.write("\n")
    with open(os.path.join(work, "timing.json"), "w") as f:
        json.dump(timing, f, indent=1)


if __name__ == "__main__":
    main()
