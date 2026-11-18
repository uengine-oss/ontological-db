#!/usr/bin/env python3
"""Probe harness for the Bolt gateway's Apache AGE backend.

Every rewrite the gateway does in front of AGE (``bolt/README.md``, "In front
of Apache AGE") has a probe here: the plain form, sent straight to AGE with
``ag_catalog.cypher()`` and *without* the rewrite. A probe answers one
question — is AGE still wrong there? (DIRECTION.md §4.2, rule 2.)

    BROKEN   AGE still answers as it did when the rewrite was written: keep it
    FIXED    AGE now gives Neo4j's answer: the rewrite may go
    CHANGED  neither of the two: look at it
    HOLDS    (relied_on probes) AGE still behaves the way the gateway relies on
    BROKEN   (relied_on probes) it no longer does: the gateway breaks
    ERROR    the harness or the probe's setup failed
    SKIPPED  not run (crashers without --include-crashers, slow, admin, ...)

Exit status: 0 when nothing is FIXED, CHANGED or ERROR and no relied_on probe
is BROKEN; 1 otherwise; 2 for a usage, probe-file or connection problem.

Python 3 standard library and the ``psql`` binary only. See README.md.
"""

import argparse
import fnmatch
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time
from datetime import datetime, timezone

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_PROBES = os.path.join(HERE, "probes.json")

KINDS = ("defect", "surface", "relied_on")
GRAPH = "@graph@"  # replaced by the probe's own graph name, agp_<id>
NULL_TEXT = "<null>"  # how psql prints SQL NULL here (-P null=...)
ID_RE = re.compile(r"^[a-z][a-z0-9_]{2,40}$")
MAX_VLE_HOPS = 4  # the most hops a variable-length pattern may have, unless only EXPLAINed

MATCHER_KEYS = {"error", "output", "output_regex", "stderr", "crash", "any"}
STEP_KEYS = {"cypher", "cols", "params", "params_var", "explain"}
PROBE_KEYS = {
    "id", "kind", "rewrite", "defects", "upstream", "source", "kills_backend", "slow",
    "gateway_only", "needs_admin", "graph", "database", "disabled", "unordered",
    "setup", "probe", "cleanup", "broken", "fixed", "note",
}

CRASH_RE = re.compile(
    r"server closed the connection unexpectedly|terminated abnormally|connection to server was lost"
    r"|terminating connection because of crash of another server process|no connection to the server"
)
CONNECT_FAIL_RE = re.compile(r"^psql: error:", re.M)
RECOVERING_RE = re.compile(r"the database system is (starting up|in recovery mode|not yet accepting connections)")
ERROR_LINE_RE = re.compile(r"^(?:psql:[^:\n]*:\d+: )?(ERROR|FATAL|PANIC):\s+(.*)$")


# ---------------------------------------------------------------- probe file

def load_probes(path):
    with open(path, encoding="utf-8") as f:
        raw = f.read()
    doc = json.loads(raw)
    return doc, hashlib.sha256(raw.encode("utf-8")).hexdigest()


def step_sql_texts(step):
    """The text a step sends (for the lints), without expanding it."""
    if isinstance(step, str):
        return [step]
    return [step.get("cypher", "")]


def is_explain_only(step):
    if isinstance(step, dict):
        return bool(step.get("explain"))
    s = step.lstrip()
    return bool(re.match(r"(?is)EXPLAIN\b", s)) and not re.search(r"(?i)\bANALY[SZ]E\b", s)


VLE_RE = re.compile(r"(?:<-|-)\[([^\[\]]*)\]")
STAR_RE = re.compile(r"\*\s*(\d*)\s*(\.\.\s*(\d*))?")


def vle_problems(text):
    """Variable-length patterns without a small upper bound."""
    out = []
    for m in VLE_RE.finditer(text):
        inner = m.group(1)
        star = STAR_RE.search(inner)
        if not star:
            continue
        lo, has_range, hi = star.group(1), star.group(2), star.group(3)
        if has_range:
            bound = int(hi) if hi else None
        else:
            bound = int(lo) if lo else None
        if bound is None:
            out.append("unbounded variable-length pattern -[%s]-" % inner)
        elif bound > MAX_VLE_HOPS:
            out.append("variable-length pattern -[%s]- has more than %d hops" % (inner, MAX_VLE_HOPS))
    return out


def check_matcher(m, where, problems):
    if isinstance(m, list):
        if not m:
            problems.append("%s: empty matcher list" % where)
        for i, x in enumerate(m):
            check_matcher(x, "%s[%d]" % (where, i), problems)
        return
    if not isinstance(m, dict) or not m:
        problems.append("%s: a matcher is a non-empty object or a list of them" % where)
        return
    bad = set(m) - MATCHER_KEYS
    if bad:
        problems.append("%s: unknown matcher keys %s" % (where, sorted(bad)))
    for k in ("error", "output_regex", "stderr"):
        if k in m:
            try:
                re.compile(m[k])
            except re.error as e:
                problems.append("%s: bad %s regex: %s" % (where, k, e))
    if "output" in m and not isinstance(m["output"], str):
        problems.append("%s: output must be a string" % where)
    if "crash" in m and m["crash"] is not True:
        problems.append("%s: crash must be true" % where)
    if "any" in m:
        check_matcher(m["any"], where + ".any", problems)


def has_crash(m):
    if isinstance(m, list):
        return any(has_crash(x) for x in m)
    if "any" in m:
        return has_crash(m["any"])
    return bool(m.get("crash"))


def check_probes(doc):
    """Problems with the probe file, as text; empty when it is fine."""
    problems = []
    if not isinstance(doc, dict) or not isinstance(doc.get("probes"), list):
        return ["the file must be an object with a list under 'probes'"]
    seen = set()
    for i, p in enumerate(doc["probes"]):
        where = "probes[%d]" % i
        if not isinstance(p, dict):
            problems.append("%s: not an object" % where)
            continue
        pid = p.get("id")
        where = "probe %s" % pid
        if not isinstance(pid, str) or not ID_RE.match(pid):
            problems.append("%s: id must match %s (it becomes the graph name agp_<id>)" % (where, ID_RE.pattern))
        if pid in seen:
            problems.append("%s: duplicate id" % where)
        seen.add(pid)
        bad = set(p) - PROBE_KEYS
        if bad:
            problems.append("%s: unknown keys %s" % (where, sorted(bad)))
        if p.get("kind") not in KINDS:
            problems.append("%s: kind must be one of %s" % (where, ", ".join(KINDS)))
        if not isinstance(p.get("rewrite"), str) or not p.get("rewrite"):
            problems.append("%s: rewrite (what it guards) is required" % where)
        for flag in ("kills_backend", "slow"):
            if not isinstance(p.get(flag), bool):
                problems.append("%s: %s must be true or false" % (where, flag))
        for flag in ("gateway_only", "needs_admin", "graph", "unordered"):
            if flag in p and not isinstance(p[flag], bool):
                problems.append("%s: %s must be true or false" % (where, flag))
        if "disabled" in p and not (isinstance(p["disabled"], str) and p["disabled"]):
            problems.append("%s: disabled must be a reason" % where)
        if p.get("gateway_only"):
            for k in ("setup", "probe", "cleanup", "broken", "fixed"):
                if p.get(k):
                    problems.append("%s: a gateway_only entry has no %s" % (where, k))
            continue
        if not p.get("probe"):
            problems.append("%s: probe (the statements sent to AGE) is required" % where)
        if "fixed" not in p:
            problems.append("%s: fixed (Neo4j's answer, or the relied-on behaviour) is required" % where)
        if p.get("kind") != "relied_on" and "broken" not in p:
            problems.append("%s: broken (how AGE answers today) is required" % where)
        for k in ("broken", "fixed"):
            if k in p:
                check_matcher(p[k], "%s.%s" % (where, k), problems)
        if p.get("kills_backend") and not has_crash(p.get("broken", {})):
            problems.append("%s: kills_backend but broken does not expect a crash" % where)
        for part in ("setup", "probe", "cleanup"):
            steps = p.get(part, [])
            if not isinstance(steps, list):
                problems.append("%s: %s must be a list" % (where, part))
                continue
            for j, st in enumerate(steps):
                sw = "%s.%s[%d]" % (where, part, j)
                if isinstance(st, dict):
                    bad = set(st) - STEP_KEYS
                    if bad:
                        problems.append("%s: unknown step keys %s" % (sw, sorted(bad)))
                    if not isinstance(st.get("cypher"), str) or not st["cypher"].strip():
                        problems.append("%s: a step object needs cypher text" % sw)
                        continue
                    if "$$" in st["cypher"]:
                        problems.append("%s: cypher text may not contain $$" % sw)
                    if "cols" in st and (not isinstance(st["cols"], int) or st["cols"] < 1):
                        problems.append("%s: cols must be a positive integer" % sw)
                    if "params" in st and "params_var" in st:
                        problems.append("%s: params or params_var, not both" % sw)
                    if "params" in st and not isinstance(st["params"], dict):
                        problems.append("%s: params must be an object" % sw)
                elif not isinstance(st, str) or not st.strip():
                    problems.append("%s: a step is SQL text or a cypher object" % sw)
                    continue
                for text in step_sql_texts(st):
                    if not is_explain_only(st):
                        for v in vle_problems(text):
                            problems.append("%s: %s (never probe with a pattern that can exhaust memory)" % (sw, v))
                    for m in re.finditer(r"\brange\(\s*(-?\d+)\s*,\s*(-?\d+)", text):
                        if int(m.group(2)) - int(m.group(1)) > 1000:
                            problems.append("%s: range() over more than 1000 values" % sw)
                    if re.search(r"(?i)\bdrop_graph\s*\(|\bdrop_label\s*\(|\bDROP\s+(DATABASE|SCHEMA|TABLE)\b", text):
                        if GRAPH not in text and "agp_" not in text and "agp\\_" not in text:
                            problems.append("%s: drops something that is not the probe's own (agp_...)" % sw)
    return problems


# ---------------------------------------------------------------- SQL text

def sql_literal(s):
    return "'" + s.replace("'", "''") + "'"


def conninfo(host, port, dbname, user):
    def q(v):
        v = str(v)
        if v and re.match(r"^[A-Za-z0-9_.\-/]+$", v):
            return v
        return "'" + v.replace("\\", "\\\\").replace("'", "\\'") + "'"
    return "host=%s port=%s dbname=%s user=%s" % (q(host), q(port), q(dbname), q(user))


class Expander:
    """Turns a probe's steps into the SQL psql is given."""

    def __init__(self, graph):
        self.graph = graph
        self.n = 0

    def sub(self, text):
        return text.replace(GRAPH, self.graph)

    def step(self, st):
        if isinstance(st, str):
            s = self.sub(st).rstrip()
            if s.startswith("\\") or s.endswith("\\gset") or s.endswith(";"):
                return s
            return s + ";"
        cols = st.get("cols", 1)
        coldefs = ", ".join("c%d ag_catalog.agtype" % i for i in range(1, cols + 1))
        with_params = "params" in st or "params_var" in st
        call = "SELECT * FROM ag_catalog.cypher(%s, $$ %s $$%s) AS (%s)" % (
            sql_literal(self.graph), self.sub(st["cypher"]), ", $1" if with_params else "", coldefs)
        explain = "EXPLAIN (COSTS OFF) " if st.get("explain") else ""
        if not with_params:
            return explain + call + ";"
        self.n += 1
        name = "agp_p%d" % self.n
        if "params" in st:
            arg = sql_literal(json.dumps(st["params"], ensure_ascii=False, sort_keys=True))
        else:
            arg = ":'%s'" % st["params_var"]
        return "PREPARE %s AS %s;\n%sEXECUTE %s(%s);" % (name, call, explain, name, arg)

    def steps(self, steps):
        return [self.step(s) for s in steps]


def session_prefix(args):
    lines = []
    if args.load_age:
        lines.append("LOAD 'age';")
    lines += [
        "SET client_min_messages = warning;",
        'SET search_path = ag_catalog, "$user", public;',
        "SET statement_timeout = %s;" % sql_literal(args.statement_timeout),
    ]
    return lines


def drop_graph_sql(graph):
    return "SELECT ag_catalog.drop_graph(name, true) FROM ag_catalog.ag_graph WHERE name = %s;" % sql_literal(graph)


def scripts_for(p, args):
    """setup, probe and cleanup scripts, as psql reads them."""
    graph = "agp_" + p["id"]
    x = Expander(graph)
    uses_graph = p.get("graph", True)
    clean = x.steps(p.get("cleanup", []))
    if uses_graph:
        clean.append(drop_graph_sql(graph))
    pre = session_prefix(args)
    setup = pre + ["\\set ON_ERROR_STOP 0"] + clean + ["\\set ON_ERROR_STOP 1"]
    if uses_graph:
        setup.append("SELECT ag_catalog.create_graph(%s);" % sql_literal(graph))
    setup += x.steps(p.get("setup", []))
    probe = pre + x.steps(p["probe"])
    cleanup = pre + ["\\set ON_ERROR_STOP 0"] + clean
    if uses_graph:
        cleanup.append("SELECT 'agp_left=' || count(*) FROM ag_catalog.ag_graph WHERE name = %s;" % sql_literal(graph))
    return graph, "\n".join(setup) + "\n", "\n".join(probe) + "\n", "\n".join(cleanup) + "\n"


# ---------------------------------------------------------------- running psql

class Outcome:
    def __init__(self, rc, stdout, stderr, timed_out=False):
        self.rc, self.stdout, self.stderr, self.timed_out = rc, stdout or "", stderr or "", timed_out
        if timed_out:
            self.kind = "timeout"
        elif CONNECT_FAIL_RE.search(self.stderr):
            self.kind = "connect"
        elif CRASH_RE.search(self.stderr):
            self.kind = "crash"
        elif rc == 0:
            # ON_ERROR_STOP is on wherever an error matters (setup, probe): psql exits 3 on one.
            # Errors before that point are the tolerant pre-cleanup's.
            self.kind = "ok"
        else:
            self.kind = "error"

    def error_text(self):
        """The first ERROR and its DETAIL / HINT lines (not the echoed SQL)."""
        lines = self.stderr.splitlines()
        for i, line in enumerate(lines):
            m = ERROR_LINE_RE.match(line)
            if m and m.group(1) == "ERROR":
                out = [m.group(2).strip()]
                for nxt in lines[i + 1:]:
                    if nxt.startswith(("DETAIL:", "HINT:", "CONTEXT:")):
                        out.append(nxt.strip())
                    elif nxt.startswith("LINE ") or not nxt.strip() or set(nxt.strip()) <= {"^"} or nxt.startswith(" "):
                        continue
                    else:
                        break
                return "\n".join(out)
        return ""

    def last_error(self):
        errs = [m.group(2).strip() for m in map(ERROR_LINE_RE.match, self.stderr.splitlines()) if m]
        return errs[-1] if errs else self.stderr.strip()[-300:]

    def summary(self):
        if self.kind == "ok":
            lines = normalize(self.stdout).splitlines()
            s = " ⏎ ".join(lines[:3]) + (" ⏎ …" if len(lines) > 3 else "")
            return s if s else "(no rows)"
        if self.kind == "error":
            return "ERROR: " + (self.error_text().splitlines() or [self.stderr.strip()[:200]])[0]
        if self.kind == "crash":
            return "backend crashed (connection lost)"
        if self.kind == "timeout":
            return "psql did not finish in time"
        first = (self.stderr.strip().splitlines() or ["(no message)"])[0]
        return "could not connect: " + first[:200]

    def as_json(self):
        return {"kind": self.kind, "rc": self.rc, "output": normalize(self.stdout),
                "error": self.error_text(), "stderr": self.stderr[-4000:]}


def normalize(text, unordered=False):
    lines = [l.rstrip() for l in text.replace("\r\n", "\n").split("\n")]
    lines = [l for l in lines if l.strip()]
    if unordered:
        lines.sort()
    return "\n".join(lines)


class Server:
    def __init__(self, args):
        self.args = args
        self.main_pw = read_password(args.password_file)
        self.admin_pw = read_password(args.admin_password_file) if args.admin_password_file else self.main_pw

    def psql(self, script, database=None, admin=False, timeout=None):
        a = self.args
        user = a.admin_user if admin else a.user
        cmd = [a.psql, "-X", "-q", "-A", "-t", "-P", "null=" + NULL_TEXT, "-v", "ON_ERROR_STOP=1",
               "-f", "-", "-d", conninfo(a.host, a.port, database or a.database, user)]
        env = dict(os.environ)
        env.update(PGCLIENTENCODING="UTF8", PGAPPNAME="age-probes", PGCONNECT_TIMEOUT="10")
        pw = self.admin_pw if admin else self.main_pw
        if pw is not None:
            env["PGPASSWORD"] = pw
        try:
            p = subprocess.run(cmd, input=script, capture_output=True, env=env, timeout=timeout or a.step_timeout,
                               encoding="utf-8", errors="replace")
            return Outcome(p.returncode, p.stdout, p.stderr)
        except subprocess.TimeoutExpired as e:
            def s(b):
                return b.decode("utf-8", "replace") if isinstance(b, bytes) else (b or "")
            return Outcome(None, s(e.stdout), s(e.stderr), timed_out=True)

    def alive(self):
        o = self.psql("SELECT 1;\n", timeout=15)
        return o.kind == "ok"

    def wait(self):
        """Wait for the server to accept connections again (after a crash)."""
        deadline = time.monotonic() + self.args.reconnect_timeout
        while time.monotonic() < deadline:
            if self.alive():
                return True
            time.sleep(1.0)
        return False


def read_password(path):
    if not path:
        return None
    with open(path, encoding="utf-8") as f:
        return f.readline().rstrip("\r\n")


# ---------------------------------------------------------------- matching

def matches(m, out, unordered):
    if isinstance(m, list):
        return any(matches(x, out, unordered) for x in m)
    if "any" in m:
        return matches(m["any"], out, unordered)
    ok = True
    if m.get("crash"):
        ok = ok and out.kind == "crash"
    if "error" in m:
        ok = ok and out.kind == "error" and re.search(m["error"], out.error_text()) is not None
    wants_rows = "output" in m or "output_regex" in m
    if wants_rows:
        # rows before an expected error count; otherwise the statements must all succeed
        ok = ok and (out.kind == "ok" or ("error" in m and out.kind == "error"))
        got = normalize(out.stdout, unordered)
        if "output" in m:
            ok = ok and got == normalize(m["output"], unordered)
        if "output_regex" in m:
            ok = ok and re.search(m["output_regex"], got) is not None
    if "stderr" in m:
        ok = ok and re.search(m["stderr"], out.stderr) is not None
    return ok


def classify(p, out):
    unordered = p.get("unordered", False)
    if p["kind"] == "relied_on":
        return "HOLDS" if matches(p["fixed"], out, unordered) else "BROKEN"
    if matches(p["broken"], out, unordered):
        return "BROKEN"
    if matches(p["fixed"], out, unordered):
        return "FIXED"
    return "CHANGED"


# ---------------------------------------------------------------- selection

def parse_only(values):
    pats = []
    for v in values or []:
        pats += [x.strip() for x in v.split(",") if x.strip()]
    return pats


def selected(p, pats):
    if not pats:
        return True
    return any(fnmatch.fnmatchcase(p["id"], pat) for pat in pats)


def named_exactly(p, pats):
    return p["id"] in pats


def skip_reason(p, args, pats):
    if p.get("gateway_only"):
        return "gateway-only: nothing AGE sees"
    if p.get("disabled") and not named_exactly(p, pats):
        return "disabled (name it in --only to run): " + p["disabled"]
    if p["kills_backend"] and not (args.include_crashers and args.disposable_server):
        return "kills the backend: needs --include-crashers --disposable-server"
    if p["slow"] and not args.include_slow:
        return "slow: needs --include-slow"
    if p.get("needs_admin") and not args.admin_user:
        return "needs --admin-user (a role that may create databases)"
    return None


def run_order(probes):
    normal = [p for p in probes if not p["slow"] and not p["kills_backend"]]
    slow = [p for p in probes if p["slow"] and not p["kills_backend"]]
    crash = [p for p in probes if p["kills_backend"]]
    return normal + slow + crash


# ---------------------------------------------------------------- the run

def base_result(p):
    return {
        "id": p["id"], "kind": p["kind"], "rewrite": p["rewrite"], "defects": p.get("defects", []),
        "upstream": p.get("upstream"), "source": p.get("source"), "kills_backend": p["kills_backend"],
        "slow": p["slow"], "gateway_only": p.get("gateway_only", False),
        "expected": {"broken": p.get("broken"), "fixed": p.get("fixed")},
        "status": None, "observed": None, "message": "", "seconds": 0.0,
    }


class ServerGone(Exception):
    pass


def run_probe(p, args, srv, pats):
    r = base_result(p)
    reason = skip_reason(p, args, pats)
    if reason:
        r["status"], r["message"] = "SKIPPED", reason
        return r
    t0 = time.monotonic()
    graph, setup, probe, cleanup = scripts_for(p, args)
    admin = bool(p.get("needs_admin"))
    msgs = []

    def after_crash(what):
        msgs.append("the backend crashed during %s" % what)
        if not srv.wait():
            raise ServerGone("the server did not come back within %ss" % args.reconnect_timeout)

    def step(script, database=None):
        o = srv.psql(script, database=database, admin=admin)
        if o.kind == "connect" and RECOVERING_RE.search(o.stderr):
            if not srv.wait():
                raise ServerGone("the server did not come back within %ss" % args.reconnect_timeout)
            o = srv.psql(script, database=database, admin=admin)
        return o

    try:
        o = step(setup)
        if o.kind == "crash":
            after_crash("setup")
        if o.kind != "ok":
            r["status"] = "ERROR"
            r["observed"] = o.as_json()
            msgs.append("setup failed: " + (o.summary() if o.kind != "error" else o.last_error()))
        else:
            o = step(probe, database=p.get("database"))
            r["observed"] = o.as_json()
            if o.kind == "crash":
                if not p["kills_backend"]:
                    msgs.append("UNEXPECTED: the backend crashed (the probe is not flagged kills_backend)")
                after_crash("the probe")
            if o.kind in ("connect", "timeout"):
                r["status"] = "ERROR"
                msgs.append(o.summary())
            else:
                r["status"] = classify(p, o)
            r["summary"] = o.summary()
    finally:
        if not args.keep_graphs:
            c = srv.psql(cleanup, admin=admin)
            if c.kind == "crash":
                after_crash("cleanup")
                c = srv.psql(cleanup, admin=admin)
            left = re.search(r"agp_left=(\d+)", c.stdout)
            if p.get("graph", True) and (not left or left.group(1) != "0"):
                msgs.append("graph %s may be left behind (cleanup: %s)" % (graph, c.summary()))
    r["message"] = "; ".join(msgs)
    r["seconds"] = round(time.monotonic() - t0, 3)
    return r


def server_info(srv):
    o = srv.psql(
        "SELECT version(), coalesce((SELECT extversion FROM pg_extension WHERE extname = 'age'), '<none>'), "
        "current_user, current_database(), current_setting('search_path'), "
        "(SELECT count(*) FROM ag_catalog.ag_graph WHERE name LIKE 'agp\\_%');\n")
    if o.kind != "ok":
        return None, o
    cells = normalize(o.stdout).split("|")
    keys = ["server", "age", "user", "database", "search_path", "agp_graphs_before"]
    return dict(zip(keys, cells)), o


# ---------------------------------------------------------------- output

STATUS_ORDER = ["FIXED", "CHANGED", "ERROR", "BROKEN", "HOLDS", "SKIPPED"]


def short(s, n):
    s = (s or "").replace("\n", " ⏎ ")
    return s if len(s) <= n else s[: n - 1] + "…"


def print_table(results, out=sys.stdout):
    w = max([len(r["id"]) for r in results] + [10])
    out.write("%-8s %-*s %-9s %s\n" % ("STATUS", w, "PROBE", "KIND", "OBSERVED"))
    for r in results:
        status = r["status"]
        if r["kind"] == "relied_on" and status == "BROKEN":
            status = "BROKEN!"
        mark = "*" if not r.get("source") and not r.get("gateway_only") else " "
        obs = r.get("summary") or r["message"]
        out.write("%-8s %-*s %-9s %s%s\n" % (status, w, r["id"] + mark, r["kind"], short(obs, 90),
                                          ("  [" + short(r["message"], 80) + "]") if r["message"] and r.get("summary") else ""))
    counts = {}
    for r in results:
        key = r["status"] if not (r["kind"] == "relied_on" and r["status"] == "BROKEN") else "BROKEN (relied_on)"
        counts[key] = counts.get(key, 0) + 1
    out.write("\n" + ", ".join("%s %d" % (k, v) for k, v in sorted(counts.items())) + "\n")
    out.write("* = the answer AGE gives today was not seen verbatim in a report; a first-run mismatch there\n"
              "    more likely means the probe's expectation needs fixing than that AGE changed.\n")
    fixed = [r for r in results if r["status"] == "FIXED"]
    if fixed:
        out.write("\nFIXED — AGE now answers as Neo4j; the rewrite may go:\n")
        for r in fixed:
            out.write("  %s: %s\n" % (r["id"], short(r["rewrite"], 150)))
    relied = [r for r in results if r["kind"] == "relied_on" and r["status"] == "BROKEN"]
    if relied:
        out.write("\nBROKEN relied-on behaviour — the gateway depends on this:\n")
        for r in relied:
            out.write("  %s: %s\n" % (r["id"], short(r["rewrite"], 150)))
    look = [r for r in results if r["status"] in ("CHANGED", "ERROR")]
    if look:
        out.write("\nLook at (CHANGED / ERROR):\n")
        for r in look:
            out.write("  %s %s: %s\n" % (r["status"], r["id"], short(r.get("summary") or r["message"], 150)))


def exit_code(results):
    for r in results:
        if r["status"] in ("FIXED", "CHANGED", "ERROR"):
            return 1
        if r["kind"] == "relied_on" and r["status"] == "BROKEN":
            return 1
    return 0


def dry_run(probes, args, pats, out=sys.stdout):
    out.write("-- dry run: the SQL each probe would send (psql -X -q -A -t -P null=%s -v ON_ERROR_STOP=1 -f -)\n"
              % NULL_TEXT)
    for p in probes:
        out.write("\n" + "=" * 78 + "\n")
        out.write("== %s  [%s]%s%s\n" % (p["id"], p["kind"], "  kills_backend" if p["kills_backend"] else "",
                                         "  slow" if p["slow"] else ""))
        out.write("-- rewrite: %s\n" % p["rewrite"])
        if p.get("defects"):
            out.write("-- defects: %s\n" % ", ".join(p["defects"]))
        if p.get("upstream"):
            out.write("-- upstream: %s\n" % p["upstream"])
        reason = skip_reason(p, args, pats)
        if reason:
            out.write("-- would be SKIPPED: %s\n" % reason)
        if p.get("gateway_only"):
            continue
        graph, setup, probe, cleanup = scripts_for(p, args)
        who = args.admin_user or "<--admin-user>" if p.get("needs_admin") else args.user
        out.write("-- [setup] as %s on database %s\n%s" % (who, args.database, setup))
        out.write("-- [probe] as %s on database %s\n%s" % (who, p.get("database") or args.database, probe))
        out.write("-- [cleanup] as %s on database %s\n%s" % (who, args.database, cleanup))
        out.write("-- broken: %s\n" % json.dumps(p.get("broken"), ensure_ascii=False))
        out.write("-- fixed:  %s\n" % json.dumps(p.get("fixed"), ensure_ascii=False))


# ---------------------------------------------------------------- main

def main(argv=None):
    ap = argparse.ArgumentParser(description="Probe Apache AGE for the defects the Bolt gateway rewrites around.")
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", default="5432")
    ap.add_argument("--database", default="graph", help="database with the age extension (default: graph)")
    ap.add_argument("--user", default="robo", help="login role; owns the graphs it creates (default: robo)")
    ap.add_argument("--password-file", help="file whose first line is the password (else PGPASSWORD / .pgpass)")
    ap.add_argument("--admin-user", help="a role that may CREATE DATABASE, for the probes that need one")
    ap.add_argument("--admin-password-file")
    ap.add_argument("--psql", default="psql", help="path of the psql binary")
    ap.add_argument("--probes", default=DEFAULT_PROBES, help="probe file (default: probes.json next to this script)")
    ap.add_argument("--only", action="append", metavar="IDS",
                    help="comma-separated probe ids or globs (repeatable); a disabled probe runs only when named exactly")
    ap.add_argument("--dry-run", action="store_true", help="print the SQL each probe would send; connect to nothing")
    ap.add_argument("--list", action="store_true", help="list the probes and exit")
    ap.add_argument("--check", action="store_true", help="validate the probe file and exit")
    ap.add_argument("--json", metavar="OUT", help="write the results as JSON")
    ap.add_argument("--include-crashers", action="store_true",
                    help="also run the probes that kill the backend (signal 11); needs --disposable-server")
    ap.add_argument("--disposable-server", action="store_true",
                    help="acknowledge that the server may be killed and restarted: a throwaway container only")
    ap.add_argument("--include-slow", action="store_true", help="also run the probes marked slow")
    ap.add_argument("--statement-timeout", default="10s")
    ap.add_argument("--step-timeout", type=float, default=120.0, help="seconds before psql itself is killed")
    ap.add_argument("--reconnect-timeout", type=float, default=120.0,
                    help="seconds to wait for the server after a crash")
    ap.add_argument("--load-age", action="store_true", help="LOAD 'age' in every session (superuser only)")
    ap.add_argument("--keep-graphs", action="store_true", help="do not drop the probe graphs (debugging)")
    args = ap.parse_args(argv)

    if args.include_crashers and not args.disposable_server:
        ap.error("--include-crashers needs --disposable-server: those probes kill the backend "
                 "(every session on the server is reset)")

    try:
        doc, digest = load_probes(args.probes)
    except (OSError, ValueError) as e:
        print("cannot read %s: %s" % (args.probes, e), file=sys.stderr)
        return 2
    problems = check_probes(doc)
    if problems:
        print("%s has problems:" % args.probes, file=sys.stderr)
        for pr in problems:
            print("  " + pr, file=sys.stderr)
        return 2
    probes = doc["probes"]
    if args.check:
        kinds = {}
        for p in probes:
            k = "gateway_only" if p.get("gateway_only") else p["kind"]
            kinds[k] = kinds.get(k, 0) + 1
        print("%s: %d probes ok (%s); kills_backend: %s" % (
            os.path.basename(args.probes), len(probes), ", ".join("%s %d" % kv for kv in sorted(kinds.items())),
            ", ".join(p["id"] for p in probes if p["kills_backend"]) or "none"))
        return 0

    pats = parse_only(args.only)
    chosen = [p for p in probes if selected(p, pats)]
    if pats and not chosen:
        print("--only matched no probe", file=sys.stderr)
        return 2
    chosen = run_order(chosen)

    if args.list:
        for p in chosen:
            flags = [f for f in ("kills_backend", "slow", "gateway_only", "needs_admin") if p.get(f)]
            if p.get("disabled"):
                flags.append("disabled")
            print("%-44s %-9s %-24s %s" % (p["id"], p["kind"], ",".join(flags), short(p["rewrite"], 70)))
        return 0
    if args.dry_run:
        dry_run(chosen, args, pats)
        return 0

    if not (os.path.isfile(args.psql) and os.access(args.psql, os.X_OK)) and not shutil.which(args.psql):
        print("psql not found: %s (give its path with --psql)" % args.psql, file=sys.stderr)
        return 2
    try:
        srv = Server(args)
    except OSError as e:
        print("cannot read a password file: %s" % e, file=sys.stderr)
        return 2
    info, o = server_info(srv)
    if info is None:
        print("cannot query the server: %s" % o.summary(), file=sys.stderr)
        if o.stderr:
            print(o.stderr.strip(), file=sys.stderr)
        return 2
    if info.get("age") == "<none>":
        print("database %s has no age extension" % args.database, file=sys.stderr)
        return 2
    print("server: %s | age %s | %s@%s | search_path %s" % (
        info["server"].split(",")[0], info["age"], info["user"], info["database"], info["search_path"]))
    if args.include_crashers:
        print("crash probes enabled: the backend will be killed and the server will reset every session.")

    started = datetime.now(timezone.utc).isoformat()
    results = []
    gone = None
    for p in chosen:
        if gone:
            r = base_result(p)
            r["status"], r["message"] = "ERROR", "not run: " + gone
            results.append(r)
            continue
        try:
            r = run_probe(p, args, srv, pats)
        except ServerGone as e:
            gone = str(e)
            r = base_result(p)
            r["status"], r["message"] = "ERROR", gone
        results.append(r)
        if r["status"] != "SKIPPED":
            print("  %-8s %s (%.1fs)" % (r["status"], r["id"], r["seconds"]), flush=True)

    print()
    order = {s: i for i, s in enumerate(STATUS_ORDER)}
    print_table(sorted(results, key=lambda r: (order.get(r["status"], 9), r["id"])))
    if args.json:
        safe_args = {k: v for k, v in vars(args).items() if "password" not in k}
        with open(args.json, "w", encoding="utf-8") as f:
            json.dump({"started": started, "finished": datetime.now(timezone.utc).isoformat(),
                       "probes_file": os.path.abspath(args.probes), "probes_sha256": digest,
                       "server": info, "args": safe_args, "results": results}, f, ensure_ascii=False, indent=2)
        print("\nresults: %s" % args.json)
    return exit_code(results)


if __name__ == "__main__":
    sys.exit(main())
