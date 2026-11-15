#!/usr/bin/env python3
"""Run the compatibility corpus over Bolt and keep (or compare) the normalised results.

    # v1 answers (the expected set), into tests/compat/expected/v1/
    python3 tests/compat/run.py --bolt-bin <ontological-bolt> --pg-port 28830 --out tests/compat/expected/v1

    # the same twice into two folders and say whether they are identical (acceptance check)
    python3 tests/compat/run.py --bolt-bin … --pg-port 28830 --repeat 2 --out /tmp/a

    # v2 against the v1 answers (once v2 has a bootstrap and a gateway)
    python3 tests/compat/run.py --target age --bolt-bin <v2-bolt> --pg-port … --out /tmp/age --compare tests/compat/expected/v1

Needs psql (for the bootstrap) and the neo4j Python driver (5 or 6). See README.md.
"""
from __future__ import annotations

import argparse
import glob
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import time
import warnings

warnings.filterwarnings("ignore", category=DeprecationWarning)

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, "extract"))

import normalize as nz  # noqa: E402
import shape  # noqa: E402

from neo4j import GraphDatabase, Query  # noqa: E402
from neo4j.exceptions import Neo4jError, DriverError  # noqa: E402

ROW_CAP = 200     # rows kept in a result file; the count and a hash cover the rest
DELTA_CAP = 100   # nodes / relationships listed per step delta


# ---------------------------------------------------------------- setup

def psql(args, sql=None, file=None, db=None, variables=None):
    cmd = [args.psql, "-h", args.pg_host, "-p", str(args.pg_port), "-U", args.pg_user, "-d", db or args.pg_database,
           "-X", "-q", "-v", "ON_ERROR_STOP=1"]
    for k, v in (variables or {}).items():
        cmd += ["-v", f"{k}={v}"]
    cmd += ["-f", file] if file else ["-c", sql]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"psql failed ({file or sql}):\n{r.stderr}")
    return r.stdout


def bootstrap(args, graphs):
    psql(args, f"DROP DATABASE IF EXISTS {args.pg_database}", db="postgres")
    psql(args, f"CREATE DATABASE {args.pg_database}", db="postgres")
    psql(args, file=os.path.join(HERE, "targets", args.target, "bootstrap.sql"), variables={"graphs": ",".join(graphs)})


def start_gateway(args, default_graph):
    if not args.bolt_bin:
        return None
    env = dict(os.environ, OG_BOLT_LISTEN=args.bolt_listen, OG_BOLT_PGHOST=args.pg_host, OG_BOLT_PGPORT=str(args.pg_port),
               OG_BOLT_PGDATABASE=args.pg_database, OG_BOLT_GRAPH=default_graph, OG_BOLT_MAX_SESSIONS="16")
    log = open(os.path.join(args.work, "bolt.log"), "a")
    gw = subprocess.Popen([args.bolt_bin], env=env, stdout=log, stderr=log)
    host, port = args.bolt_listen.split(":")
    for _ in range(200):
        try:
            socket.create_connection((host, int(port)), 0.5).close()
            return gw
        except OSError:
            time.sleep(0.1)
    gw.kill()
    sys.exit("the Bolt gateway did not start")


def stop_gateway(gw):
    if gw is None:
        return
    gw.terminate()
    try:
        gw.wait(10)
    except subprocess.TimeoutExpired:
        gw.kill()


# ---------------------------------------------------------------- corpus

def load_corpus(clients):
    out = {}
    for c in clients:
        d = os.path.join(HERE, "corpus", c)
        man = json.load(open(os.path.join(d, "manifest.json"), encoding="utf-8"))
        for q in man["queries"]:
            q["text"] = open(os.path.join(d, "queries", q["id"] + ".cypher"), encoding="utf-8").read().rstrip("\n")
            pf = os.path.join(d, "params", q["id"] + ".json")
            q["param_spec"] = json.load(open(pf, encoding="utf-8")) if os.path.exists(pf) else {}
        out[c] = (d, man)
    return out


def fixture_statements(path, conf_graph, subst):
    sys.path.insert(0, os.path.join(HERE, "extract"))
    from extract import split_statements
    text = open(path, encoding="utf-8").read()
    m = re.search(r"^//\s*graph:\s*(\S+)", text, re.M)
    graph = m.group(1) if m else conf_graph
    for k, v in subst.items():
        text = text.replace("{{" + k + "}}", v)
    return graph, split_statements(text)


# ---------------------------------------------------------------- running one statement

class Runner:
    def __init__(self, args):
        self.args = args
        self.driver = GraphDatabase.driver(args.bolt_uri, auth=(args.pg_user, args.pg_password))
        self.env = {"COMPAT_PGHOST": args.pg_host, "COMPAT_PGPORT": str(args.pg_port),
                    "COMPAT_PGDATABASE": args.pg_database, "COMPAT_PGUSER": args.pg_user}

    def close(self):
        self.driver.close()

    def run_auto(self, graph, text, params=None):
        with self.driver.session(database=graph) as s:
            res = s.run(Query(text, timeout=self.args.timeout), params or {})
            keys = list(res.keys())
            records = list(res)
            summary = res.consume()
        return keys, records, summary

    # -- known ids of a graph ------------------------------------------------
    def digest(self, run):
        """run(text) -> (keys, records). Returns (nodes{eid: raw}, rels{eid: raw})."""
        nodes, rels = {}, {}
        _, recs = run("MATCH (n) RETURN n")[:2]
        for r in recs:
            n = r["n"]
            nodes[n.element_id] = n
        _, recs = run("MATCH ()-[r]->() RETURN r")[:2]
        for r in recs:
            rels[r["r"].element_id] = r["r"]
        return nodes, rels

    def known_of(self, nodes, rels):
        known = {}
        for eid, n in nodes.items():
            known[eid] = "n"
            known[str(getattr(n, "id", eid))] = "n"
        for eid, r in rels.items():
            known[eid] = "r"
            known[str(getattr(r, "id", eid))] = "r"
        return known

    # -- parameters ---------------------------------------------------------
    def resolve(self, spec, run):
        """Fill {"$ref": …} / {"$env": …} with values read from the graph / the connection."""
        if isinstance(spec, list):
            out = [self.resolve(x, run) for x in spec]
            # a looked-up value that is not there is left out of a list (not sent as null)
            return [v for v, s in zip(out, spec) if v is not None or not (isinstance(s, dict) and "$ref" in s)]
        if not isinstance(spec, dict):
            return spec
        if "$env" in spec:
            v = self.env.get(spec["$env"], "")
            return int(v) if spec.get("type") == "int" else v
        if "$ref" in spec and isinstance(spec.get("$ref"), str):
            kind, label, nth = spec["$ref"], spec.get("label"), int(spec.get("nth", 0))
            m = f"(n:`{label}`)" if label and label != "*" else "(n)"
            order = ("toString(coalesce(n.id, n._source_id, n.key, n.schema_id, n.class_name, n.chunk_id, "
                     "n.document_id, n.name)), toString(properties(n))")
            if kind == "prop":
                q = f"MATCH {m} WHERE n.`{spec['prop']}` IS NOT NULL RETURN DISTINCT n.`{spec['prop']}` AS v ORDER BY v SKIP {nth} LIMIT 1"
            elif kind == "anyprop":
                q = f"MATCH (n) WHERE n.`{spec['prop']}` IS NOT NULL RETURN DISTINCT n.`{spec['prop']}` AS v ORDER BY v SKIP {nth} LIMIT 1"
            elif kind in ("elementId", "id"):
                fn = "elementId" if kind == "elementId" else "id"
                q = f"MATCH {m} WITH n ORDER BY {order} SKIP {nth} LIMIT 1 RETURN {fn}(n) AS v"
            else:
                return None
            try:
                _, recs = run(q)[:2]
            except Neo4jError:
                return None
            return recs[0]["v"] if recs else None
        return {k: self.resolve(v, run) for k, v in spec.items()}


# ---------------------------------------------------------------- normalising a result

def id_columns(info):
    return {alias for expr, alias in info["columns"] if re.search(r"\b(elementId|id)\s*\(", expr)}


def sorted_columns(info):
    return {alias for expr, alias in info["columns"] if re.search(r"\b(collect|labels|keys)\s*\(", expr, re.I)}


def limit_of(text, params):
    """The number after the last top-level LIMIT (a literal or a parameter), or None."""
    m = shape.mask_strings(text)
    pos = shape._top_level_positions(m, "LIMIT")
    if not pos:
        return None
    g = re.match(r"LIMIT\s+(\d+|\$(\w+))", m[pos[-1]:], re.I)
    if not g:
        return None
    if g.group(2):
        v = (params or {}).get(g.group(2))
        return v if isinstance(v, int) else None
    return int(g.group(1))


def shape_result(norm: nz.Normalizer, text, keys, records, summary=None, params=None):
    info = shape.last_return(text)
    idc, sortc = id_columns(info), sorted_columns(info)
    if any(re.match(r"^\s*(id|elementId)\s*\(", expr) or expr == "*" for expr, _ in info["columns"]):
        pass
    raw_rows = []
    for rec in records:
        row = []
        for k in keys:
            v = norm.raw(rec[k], id_column=k in idc)
            if k in sortc and not info["ordered_collect"]:
                v = norm.sort_lists(v)
            row.append(v)
        raw_rows.append(row)
    ordered = info["ordered"]
    ties = None
    if not ordered:
        raw_rows.sort(key=lambda r: nz.canon(norm.mask(r)))
    else:
        # ORDER BY leaves rows with equal keys in any order: when every key is a returned column,
        # rows with equal keys are sorted among themselves
        cols = [c for c, _ in info["columns"]]
        aliases = [a for _, a in info["columns"]]
        idx = []
        for e in info["order_by"]:
            if e in aliases:
                idx.append(aliases.index(e))
            elif e in cols:
                idx.append(cols.index(e))
            else:
                idx = None
                break
        if idx:
            ties = "sorted"
            out_rows, group = [], []
            for row in raw_rows:
                if group and [nz.canon(norm.mask(group[0][i])) for i in idx] != [nz.canon(norm.mask(row[i])) for i in idx]:
                    out_rows += sorted(group, key=lambda r: nz.canon(norm.mask(r)))
                    group = []
                group.append(row)
            out_rows += sorted(group, key=lambda r: nz.canon(norm.mask(r)))
            raw_rows = out_rows
        else:
            ties = "server"
    rows = [norm.finish(r) for r in raw_rows]
    out = {"columns": list(keys), "row_order": ("server, ties " + ties) if ordered else "sorted", "row_count": len(rows)}
    lim = limit_of(text, params)
    if info["limit"] and not info["ordered"] and (lim is None or len(rows) >= lim):
        # LIMIT without ORDER BY cut the result: which rows came back is not defined
        out["subset"] = "LIMIT without ORDER BY cut the result; only columns and row_count are kept"
        rows = []
    if len(rows) > ROW_CAP:
        out["rows_sha1"] = nz.sha1(rows)
        out["rows_truncated"] = True
        rows = rows[:ROW_CAP]
    out["rows"] = rows
    if summary is not None:
        c = {k: v for k, v in vars(summary.counters).items() if not k.startswith("_") and v and k != "contains_updates"
             and k != "contains_system_updates"}
        if c:
            out["counters"] = dict(sorted(c.items()))
    return out


def node_key(norm, n):
    return nz.canon({"labels": sorted(n.labels), "props": norm.finish(norm.mask({k: norm.raw(v) for k, v in sorted(dict(n).items())}))})


def delta(norm, before, after):
    bn, br = before
    an, ar = after
    keyof = {}
    for eid, n in list(bn.items()) + list(an.items()):
        keyof[eid] = node_key(norm, n)
    added = sorted(keyof[e] for e in an if e not in bn)
    removed = sorted(keyof[e] for e in bn if e not in an)
    changed = sorted(f"{keyof[e]} <- {node_key(norm, bn[e])}" for e in an if e in bn and node_key(norm, bn[e]) != keyof[e])

    def rkey(r):
        return nz.canon({"type": r.type, "start": keyof.get(r.start_node.element_id, "?"),
                         "end": keyof.get(r.end_node.element_id, "?"),
                         "props": norm.finish(norm.mask({k: norm.raw(v) for k, v in sorted(dict(r).items())}))})
    radded = sorted(rkey(r) for e, r in ar.items() if e not in br)
    rremoved = sorted(rkey(r) for e, r in br.items() if e not in ar)
    out = {}
    for name, lst in (("nodes_added", added), ("nodes_removed", removed), ("nodes_changed", changed),
                      ("rels_added", radded), ("rels_removed", rremoved)):
        if lst:
            out[name] = [json.loads(x) if not " <- " in x else x for x in lst[:DELTA_CAP]]
            if len(lst) > DELTA_CAP:
                out[name + "_count"] = len(lst)
    return out


# ---------------------------------------------------------------- one full pass

def load_unstable():
    path = os.path.join(HERE, "unstable.json")
    if not os.path.exists(path):
        return {}
    return {u["id"]: u["reason"] for u in json.load(open(path, encoding="utf-8")) if not u["id"].startswith("_")}


def one_pass(args, out_dir):
    clients = args.clients
    corpus = load_corpus(clients)
    unstable = load_unstable()
    graphs = sorted({g for _, man in corpus.values() for g in man["graphs"]})
    bootstrap(args, graphs)
    gw = start_gateway(args, graphs[0])
    runner = Runner(args)
    summary = {"target": args.target, "clients": {}}
    subst = {"PGHOST": args.pg_host, "PGPORT": str(args.pg_port), "PGDATABASE": args.pg_database, "PGUSER": args.pg_user}
    try:
        if os.path.isdir(out_dir):
            shutil.rmtree(out_dir)
        os.makedirs(out_dir)
        # fixtures: shared first, then each client's, in file order
        setup = []
        files = [(os.path.join(HERE, "corpus", "_shared", "fixture"), graphs[0])]
        conf = json.load(open(os.path.join(HERE, "extract", "clients.json"), encoding="utf-8"))
        files += [(os.path.join(d, "fixture"), conf[c]["graph"]) for c, (d, man) in corpus.items()]
        for fdir, g0 in files:
            for f in sorted(glob.glob(os.path.join(fdir, "*.cypher"))):
                graph, stmts = fixture_statements(f, g0, subst)
                if graph == "*":
                    graph = graphs[0]
                ok = failed = 0
                errors = []
                for line, stmt in stmts:
                    try:
                        runner.run_auto(graph, stmt)
                        ok += 1
                    except Neo4jError as e:
                        failed += 1
                        errors.append({"line": line, **nz.error_of(e)})
                setup.append({"file": os.path.relpath(f, HERE), "graph": graph, "statements": ok + failed,
                              "failed": failed, "errors": errors})
        dump(os.path.join(out_dir, "_setup.json"), setup)

        for c, (d, man) in corpus.items():
            cdir = os.path.join(out_dir, c)
            os.makedirs(os.path.join(cdir, "scenarios"))
            stats = {"reads": 0, "reads_ok": 0, "steps": 0, "steps_ok": 0, "errors": {}}
            failing = []
            byid = {q["id"]: q for q in man["queries"]}
            # graph state the reads see
            states = {}
            for g in man["graphs"]:
                states[g] = runner.digest(lambda t, g=g: runner.run_auto(g, t))
            # ---- reads
            for q in man["queries"]:
                if q["kind"] not in ("read", "fabric-read"):
                    continue
                g = q["graph"]
                norm = nz.Normalizer(runner.known_of(*states[g]))
                runf = lambda t, p=None, g=g: runner.run_auto(g, t, p)  # noqa: E731
                params = runner.resolve(q["param_spec"], runf)
                res = {"id": q["id"], "graph": g, "kind": q["kind"]}
                try:
                    keys, recs, summ = runf(q["text"], params)
                    res.update(shape_result(norm, q["text"], keys, recs, summ, params))
                    stats["reads_ok"] += 1
                except (Neo4jError, DriverError) as e:
                    res["error"] = nz.error_of(e)
                    stats["errors"][res["error"]["code"]] = stats["errors"].get(res["error"]["code"], 0) + 1
                    failing.append({"id": q["id"], "code": res["error"]["code"]})
                stats["reads"] += 1
                if q["id"] in unstable:
                    res = {"id": q["id"], "graph": g, "kind": q["kind"], "unstable": unstable[q["id"]]}
                if params:
                    res["params"] = norm.finish(norm.raw(params))
                dump(os.path.join(cdir, q["id"] + ".json"), res)
            # ---- write scenarios, each in a transaction that is rolled back
            for sc in man["scenarios"]:
                g = sc["graph"]
                norm = nz.Normalizer()
                result = {"scenario": sc["name"], "graph": g, "isolation": sc["isolation"], "steps": []}
                done = []   # (text, params) of steps that went through, to replay after a failure
                with runner.driver.session(database=g) as s:
                    tx = s.begin_transaction(timeout=args.timeout)
                    txrun = lambda t, p=None: _txrun(tx, t, p)  # noqa: E731
                    before = runner.digest(lambda t: txrun(t))
                    norm.learn(runner.known_of(*before))
                    for sid in sc["steps"]:
                        q = byid[sid]
                        step = {"id": sid}
                        params = runner.resolve(q["param_spec"], lambda t: txrun(t))
                        try:
                            keys, recs, summ = txrun(q["text"], params)
                            after = runner.digest(lambda t: txrun(t))
                            norm.learn(runner.known_of(*after))
                            step.update(shape_result(norm, q["text"], keys, recs, summ, params))
                            step["effect"] = delta(norm, before, after)
                            before = after
                            done.append((q["text"], params))
                            stats["steps_ok"] += 1
                        except (Neo4jError, DriverError) as e:
                            step["error"] = nz.error_of(e)
                            stats["errors"][step["error"]["code"]] = stats["errors"].get(step["error"]["code"], 0) + 1
                            failing.append({"id": sid, "code": step["error"]["code"], "scenario": sc["name"]})
                            try:
                                tx.rollback()
                            except Exception:  # noqa: BLE001 - already closed by the failure
                                pass
                            tx = s.begin_transaction(timeout=args.timeout)
                            txrun = lambda t, p=None, tx=tx: _txrun(tx, t, p)  # noqa: E731
                            for t, p in done:
                                txrun(t, p)
                            before = runner.digest(lambda t: txrun(t))
                        stats["steps"] += 1
                        if sid in unstable:
                            step = {"id": sid, "unstable": unstable[sid]}
                        if params:
                            step["params"] = norm.finish(norm.raw(params))
                        result["steps"].append(step)
                    try:
                        tx.rollback()
                    except Exception:  # noqa: BLE001
                        pass
                dump(os.path.join(cdir, "scenarios", slug(sc["name"]) + ".json"), result)
            stats["errors"] = dict(sorted(stats["errors"].items()))
            summary["clients"][c] = {**stats, "failing": sorted(failing, key=lambda x: x["id"])}
        dump(os.path.join(out_dir, "summary.json"), summary)
    finally:
        runner.close()
        stop_gateway(gw)
    return summary


def _txrun(tx, text, params=None):
    res = tx.run(text, params or {})
    keys = list(res.keys())
    recs = list(res)
    summ = res.consume()
    return keys, recs, summ


def slug(name):
    return re.sub(r"[^A-Za-z0-9_.@-]+", "__", name).strip("_")


def dump(path, obj):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        json.dump(obj, f, ensure_ascii=False, indent=1)
        f.write("\n")


# ---------------------------------------------------------------- comparing two result sets

def compare(got_dir, want_dir, known_path=None):
    """Differences between two result folders, with the relaxations the expected files ask for."""
    known = {}
    if known_path and os.path.exists(known_path):
        for k in json.load(open(known_path, encoding="utf-8")):
            known[k["id"]] = k
    diffs = []
    for want in sorted(glob.glob(os.path.join(want_dir, "*", "*.json")) + glob.glob(os.path.join(want_dir, "*", "scenarios", "*.json"))):
        rel = os.path.relpath(want, want_dir)
        got = os.path.join(got_dir, rel)
        w = json.load(open(want, encoding="utf-8"))
        if not os.path.exists(got):
            diffs.append({"file": rel, "why": "missing"})
            continue
        g = json.load(open(got, encoding="utf-8"))
        key = w.get("id") or w.get("scenario")
        if key in known:
            continue
        if not same(w, g):
            diffs.append({"file": rel, "why": "different"})
    return diffs


def same(w, g):
    if "steps" in w:
        return len(w["steps"]) == len(g.get("steps", [])) and all(same(a, b) for a, b in zip(w["steps"], g["steps"]))
    if "error" in w or "error" in g:
        return ("error" in w) == ("error" in g) and (w.get("error") or {}).get("code") == (g.get("error") or {}).get("code")
    if w.get("subset"):
        return w.get("columns") == g.get("columns") and w.get("row_count") == g.get("row_count")
    keys = ("columns", "row_count", "rows", "rows_sha1", "effect")
    return all(w.get(k) == g.get(k) for k in keys)


# ---------------------------------------------------------------- main

def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--target", default="v1", choices=["v1", "age"], help="which bootstrap (targets/<target>/bootstrap.sql)")
    ap.add_argument("--clients", nargs="*", default=["studio", "robo", "analyzer"])
    ap.add_argument("--pg-host", default=os.environ.get("PGHOST", "127.0.0.1"))
    ap.add_argument("--pg-port", type=int, default=int(os.environ.get("PGPORT", "28830")))
    ap.add_argument("--pg-user", default=os.environ.get("PGUSER", "og"))
    ap.add_argument("--pg-password", default=os.environ.get("PGPASSWORD", ""))
    ap.add_argument("--pg-database", default="og_compat", help="dropped and made again at every pass")
    ap.add_argument("--psql", default=os.environ.get("PSQL", "psql"))
    ap.add_argument("--bolt-bin", default=os.environ.get("OG_BOLT_BIN"), help="start this gateway (else --bolt-uri must be up)")
    ap.add_argument("--bolt-listen", default="127.0.0.1:28731")
    ap.add_argument("--bolt-uri", default=None)
    ap.add_argument("--timeout", type=float, default=30.0, help="seconds per statement")
    ap.add_argument("--out", required=True, help="result folder (replaced)")
    ap.add_argument("--repeat", type=int, default=1, help="run N passes into <out>, <out>.2 … and check they are identical")
    ap.add_argument("--compare", help="expected folder to compare the (first) pass with")
    ap.add_argument("--known-differences", default=os.path.join(HERE, "expected", "known-differences.json"))
    ap.add_argument("--work", default=None, help="folder for the gateway log (default: next to --out)")
    args = ap.parse_args()
    args.bolt_uri = args.bolt_uri or f"bolt://{args.bolt_listen}"
    args.work = args.work or os.path.dirname(os.path.abspath(args.out))
    os.makedirs(args.work, exist_ok=True)

    outs = [args.out] + [f"{args.out}.{i}" for i in range(2, args.repeat + 1)]
    summaries = []
    for i, out in enumerate(outs):
        t0 = time.time()
        s = one_pass(args, out)
        summaries.append(s)
        tot = {k: sum(c[k] for c in s["clients"].values()) for k in ("reads", "reads_ok", "steps", "steps_ok")}
        print(f"pass {i + 1}: {out}  reads {tot['reads_ok']}/{tot['reads']} ok, steps {tot['steps_ok']}/{tot['steps']} ok"
              f"  ({time.time() - t0:.0f}s)")
    rc = 0
    if len(outs) > 1:
        for other in outs[1:]:
            r = subprocess.run(["diff", "-r", "-q", outs[0], other], capture_output=True, text=True)
            if r.returncode == 0:
                print(f"identical: {outs[0]} == {other}")
            else:
                rc = 1
                print(f"DIFFERENT: {outs[0]} vs {other}\n{r.stdout}")
    if args.compare:
        diffs = compare(args.out, args.compare, args.known_differences)
        print(f"compare with {args.compare}: {len(diffs)} differences")
        for d in diffs[:50]:
            print(f"  {d['file']}: {d['why']}")
        rc = rc or (1 if diffs else 0)
    sys.exit(rc)


if __name__ == "__main__":
    main()
