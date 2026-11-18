#!/usr/bin/env python3
"""Time a client's part of the answer set on several targets in one run, alternating.

Every target is loaded with the same fixture (run.py's bootstrap, gateway and fixture code). Then every read
query, and every write scenario (in a transaction that is rolled back), runs `--warmup` + `--reps` times on each
target, the targets taking turns in a rotating order within each repetition, so a busy machine slows all of them
alike. Times are client-side: from sending the statement to the summary (the whole result fetched).

    python3 tests/compat/bench.py --config bench.json --clients robo --out /tmp/bench.json

bench.json: {"targets": [{"name": "v1", "target": "v1", "pg_host": …, "pg_port": …, "pg_user": …,
  "pg_password_file": …, "pg_database": …, "db_locale": "C", "bolt_bin": …, "bolt_listen": …,
  "bolt_env": ["K=V", …], "psql": …, "bootstrap_user": null, "bootstrap_password_file": null,
  "reuse_db": false}, …],
  "also": [{"name": "age-nocount", "same_as": "age", "bolt_listen": …, "bolt_env": [...], "writes_only": true},
           {"name": "age-trace", "same_as": "age", "bolt_listen": …, "bolt_env": ["OG_BOLT_AGE_TRACE=1"],
            "classify": true}]}

"also" entries are extra gateways on a loaded target's database (not loaded again): timed too (writes only, if
writes_only), or — classify — run once per statement with their log read to tell each statement's plan
(OG_BOLT_AGE_TRACE: cypher / steps / reach …) and how many cypher() calls it took.
"""
from __future__ import annotations

import argparse
import glob
import json
import os
import sys
import time
from types import SimpleNamespace

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import run as rp  # noqa: E402

from neo4j import Query  # noqa: E402
from neo4j.exceptions import DriverError, Neo4jError  # noqa: E402


def target_args(t, work, timeout, clients):
    pw = open(t["pg_password_file"], encoding="utf-8").read().strip() if t.get("pg_password_file") else ""
    a = SimpleNamespace(
        target=t.get("target", "v1"), clients=clients, pg_host=t.get("pg_host", "127.0.0.1"), pg_port=int(t["pg_port"]),
        pg_user=t["pg_user"], pg_password=pw, bootstrap_user=t.get("bootstrap_user"), pg_database=t["pg_database"],
        psql=t.get("psql", "psql"), db_locale=t.get("db_locale"), reuse_db=bool(t.get("reuse_db")),
        bolt_bin=t["bolt_bin"], bolt_listen=t["bolt_listen"], bolt_env=list(t.get("bolt_env", [])),
        timeout=timeout, work=os.path.join(work, t["name"]), skip_shared=True)
    a.bolt_uri = f"bolt://{a.bolt_listen}"
    os.makedirs(a.work, exist_ok=True)
    a.admin_password = (open(t["bootstrap_password_file"], encoding="utf-8").read().strip()
                        if t.get("bootstrap_password_file") else pw)
    return a


def load(a, corpus, graphs):
    """bootstrap, gateway, fixture, statistics — as one pass of run.py does."""
    os.environ["PGPASSWORD"] = a.admin_password
    rp.bootstrap(a, graphs)
    gw = rp.start_gateway(a, graphs[0])
    runner = rp.Runner(a)
    subst = {"PGHOST": a.pg_host, "PGPORT": str(a.pg_port), "PGDATABASE": a.pg_database, "PGUSER": a.pg_user}
    conf = json.load(open(os.path.join(HERE, "extract", "clients.json"), encoding="utf-8"))
    failed = 0
    for c, (d, man) in corpus.items():
        for f in sorted(glob.glob(os.path.join(d, "fixture", "*.cypher"))):
            graph, stmts = rp.fixture_statements(f, conf[c]["graph"], subst)
            if graph == "*":
                graph = graphs[0]
            for _, stmt in stmts:
                try:
                    runner.run_auto(graph, stmt)
                except Neo4jError:
                    failed += 1
    rp.psql(a, file=os.path.join(HERE, "targets", "stats.sql"))
    return gw, runner, failed


class Timed:
    """A target's driver, with a session kept open per graph."""

    def __init__(self, name, runner, timeout):
        self.name, self.runner, self.timeout = name, runner, timeout
        self.sessions = {}

    def session(self, g):
        if g not in self.sessions:
            self.sessions[g] = self.runner.driver.session(database=g)
        return self.sessions[g]

    def run(self, g, text, params):
        s = self.session(g)
        t0 = time.perf_counter()
        res = s.run(Query(text, timeout=self.timeout), params or {})
        n = sum(1 for _ in res)
        res.consume()
        return (time.perf_counter() - t0) * 1000.0, n

    def close(self):
        for s in self.sessions.values():
            s.close()


def err_code(e):
    return getattr(e, "code", None) or type(e).__name__


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--config", required=True)
    ap.add_argument("--clients", nargs="*", default=["robo"])
    ap.add_argument("--warmup", type=int, default=2)
    ap.add_argument("--reps", type=int, default=10)
    ap.add_argument("--timeout", type=float, default=30.0, help="seconds per statement; a target that times out on "
                    "a statement's first run is not asked it again")
    ap.add_argument("--only", nargs="*", default=None, help="only these query ids / scenario names")
    ap.add_argument("--skip-scenarios", action="store_true")
    ap.add_argument("--work", default=None)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    conf = json.load(open(args.config, encoding="utf-8"))
    work = args.work or os.path.dirname(os.path.abspath(args.out))
    corpus = rp.load_corpus(args.clients)
    graphs = sorted({g for _, man in corpus.values() for g in man["graphs"]})
    loaded, gws, timed, classify = {}, [], [], []
    out = {"started": time.strftime("%Y-%m-%dT%H:%M:%S"), "warmup": args.warmup, "reps": args.reps, "targets": [],
           "reads": {}, "steps": {}, "plans": {}, "floor": {}}
    try:
        for t in conf["targets"]:
            a = target_args(t, work, args.timeout, args.clients)
            t0 = time.time()
            gw, runner, failed = load(a, corpus, graphs)
            gws.append(gw)
            loaded[t["name"]] = (a, runner)
            timed.append((Timed(t["name"], runner, args.timeout), False))
            out["targets"].append({"name": t["name"], "fixture_failed": failed, "load_s": round(time.time() - t0, 1)})
            print(f"loaded {t['name']}: {round(time.time() - t0, 1)} s, fixture statements failed {failed}", flush=True)
        for x in conf.get("also", []):
            base, _ = loaded[x["same_as"]]
            a = SimpleNamespace(**vars(base))
            a.bolt_listen, a.bolt_env = x["bolt_listen"], list(base.bolt_env) + list(x.get("bolt_env", []))
            a.bolt_uri = f"bolt://{a.bolt_listen}"
            a.work = os.path.join(work, x["name"])
            os.makedirs(a.work, exist_ok=True)
            log = os.path.join(a.work, "bolt.log")
            if os.path.exists(log):
                os.remove(log)
            gw = rp.start_gateway(a, graphs[0])
            gws.append(gw)
            r = rp.Runner(a)
            if x.get("classify"):
                classify.append((x["name"], Timed(x["name"], r, args.timeout), log, base))
            else:
                timed.append((Timed(x["name"], r, args.timeout), bool(x.get("writes_only"))))
            out["targets"].append({"name": x["name"], "same_as": x["same_as"], "env": x.get("bolt_env", []),
                                   "writes_only": bool(x.get("writes_only")), "classify": bool(x.get("classify"))})
        names = [tm.name for tm, _ in timed]
        # the floor: RETURN 1, per target, taking turns like everything else
        for tm, _ in timed:
            out["floor"][tm.name] = []
        for i in range(args.warmup + 30):
            k = i % len(timed)
            for tm, _ in timed[k:] + timed[:k]:
                ms = tm.run(graphs[0], "RETURN 1 AS x", {})[0]
                if i >= args.warmup:
                    out["floor"][tm.name].append(ms)
        # ---- reads
        for c, (d, man) in corpus.items():
            reads = [q for q in man["queries"] if q["kind"] == "read" and (args.only is None or q["id"] in args.only)]
            params = {}
            for tm, _ in timed:
                for q in reads:
                    runf = lambda t_, p=None, g=q["graph"], tm=tm: tm.runner.run_auto(g, t_, p)  # noqa: E731
                    params[(tm.name, q["id"])] = tm.runner.resolve(q["param_spec"], runf)
            for qi, q in enumerate(reads):
                rec = {n: {"ms": [], "rows": None, "error": None} for n, (tm, wo) in zip(names, timed) if not wo}
                skip = set()
                for rep in range(args.warmup + args.reps):
                    order = [(tm, wo) for tm, wo in timed if not wo]
                    k = rep % len(order)
                    for tm, _ in order[k:] + order[:k]:
                        if tm.name in skip:
                            continue
                        try:
                            ms, n = tm.run(q["graph"], q["text"], params[(tm.name, q["id"])])
                            rec[tm.name]["rows"] = n
                        except (Neo4jError, DriverError) as e:
                            ms = None
                            rec[tm.name]["error"] = err_code(e)
                            if "Timed" in str(rec[tm.name]["error"]) or "Timeout" in str(rec[tm.name]["error"]):
                                skip.add(tm.name)
                        if rep >= args.warmup and ms is not None:
                            rec[tm.name]["ms"].append(ms)
                out["reads"][q["id"]] = rec
                if qi % 50 == 0:
                    print(f"reads {qi}/{len(reads)}", flush=True)
            # ---- scenarios: every step timed, in a rolled-back transaction per repetition
            if not args.skip_scenarios:
                byid = {q["id"]: q for q in man["queries"]}
                scs = [s for s in man["scenarios"] if args.only is None or s["name"] in args.only]
                for si, sc in enumerate(scs):
                    for rep in range(args.warmup + args.reps):
                        k = rep % len(timed)
                        for tm, _ in timed[k:] + timed[:k]:
                            with tm.runner.driver.session(database=sc["graph"]) as s:
                                tx = s.begin_transaction(timeout=args.timeout)
                                done = []
                                for i, sid in enumerate(sc["steps"]):
                                    q = byid[sid]
                                    p = tm.runner.resolve(q["param_spec"], lambda t_, tx=tx: rp._txrun(tx, t_))
                                    key = f"{sc['name']}#{i}:{sid}"
                                    slot = out["steps"].setdefault(key, {}).setdefault(tm.name, {"ms": [], "error": None})
                                    t0 = time.perf_counter()
                                    try:
                                        res = tx.run(q["text"], p or {})
                                        for _ in res:
                                            pass
                                        res.consume()
                                        if rep >= args.warmup:
                                            slot["ms"].append((time.perf_counter() - t0) * 1000.0)
                                        done.append((q["text"], p))
                                    except (Neo4jError, DriverError) as e:
                                        slot["error"] = err_code(e)
                                        try:
                                            tx.rollback()
                                        except Exception:  # noqa: BLE001
                                            pass
                                        tx = s.begin_transaction(timeout=args.timeout)
                                        for t_, p_ in done:
                                            rp._txrun(tx, t_, p_)
                                try:
                                    tx.rollback()
                                except Exception:  # noqa: BLE001
                                    pass
                    if si % 20 == 0:
                        print(f"scenarios {si}/{len(scs)}", flush=True)
            # ---- plans, from the trace gateway's log
            for cname, tm, log, base in classify:
                def plans_of(fn):
                    size = os.path.getsize(log) if os.path.exists(log) else 0
                    err = None
                    try:
                        fn()
                    except (Neo4jError, DriverError) as e:
                        err = err_code(e)
                    time.sleep(0.01)
                    with open(log, encoding="utf-8", errors="replace") as f:
                        f.seek(size)
                        lines = f.read().splitlines()
                    plans = [ln.split("plan ", 1)[1] for ln in lines if ln.startswith("age-trace plan ")]
                    calls = sum(1 for ln in lines if ln.startswith("age-trace > "))
                    return {"plans": plans, "cypher_calls": calls, "error": err}
                for q in [q for q in man["queries"] if q["kind"] == "read" and (args.only is None or q["id"] in args.only)]:
                    runf = lambda t_, p=None, g=q["graph"]: tm.runner.run_auto(g, t_, p)  # noqa: E731
                    p = tm.runner.resolve(q["param_spec"], runf)
                    out["plans"][q["id"]] = plans_of(lambda: tm.run(q["graph"], q["text"], p))
                if not args.skip_scenarios:
                    for sc in [s for s in man["scenarios"] if args.only is None or s["name"] in args.only]:
                        with tm.runner.driver.session(database=sc["graph"]) as s:
                            tx = s.begin_transaction(timeout=args.timeout)
                            for i, sid in enumerate(sc["steps"]):
                                q = byid[sid]
                                p = tm.runner.resolve(q["param_spec"], lambda t_, tx=tx: rp._txrun(tx, t_))
                                r = plans_of(lambda: rp._txrun(tx, q["text"], p))
                                out["plans"][f"{sc['name']}#{i}:{sid}"] = r
                                if r["error"]:
                                    try:
                                        tx.rollback()
                                    except Exception:  # noqa: BLE001
                                        pass
                                    tx = s.begin_transaction(timeout=args.timeout)
                            try:
                                tx.rollback()
                            except Exception:  # noqa: BLE001
                                pass
    finally:
        for tm, _ in timed:
            tm.close()
        for _, (a, runner) in loaded.items():
            runner.close()
        for gw in gws:
            rp.stop_gateway(gw)
        out["finished"] = time.strftime("%Y-%m-%dT%H:%M:%S")
        with open(args.out, "w", encoding="utf-8") as f:
            json.dump(out, f)
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
