#!/usr/bin/env python3
"""Build docs/tutorials/virtual-fabric-tutorial.md by *running* tutorial.src.md.

Every statement in the tutorial is sent to the Ontological Studio the way its
editor sends it — POST /api/cypher with the graph, the text and the parameters
set by `:param` — and what came back is written under it. Nothing in the
published tutorial's output is typed by hand: if an example stops working, this
script fails. Marked results are then replayed in the Studio's page itself, in
a real browser, and photographed.

Directives (after the language on the fence line):

    ```cypher run         run each statement, show each result
    ```cypher run silent  run, show nothing
    ```cypher error       must fail; show the error
    ```cypher sql         show what the Studio's SQL tab shows for it
    ```python run         run with the container's python (Bolt example)

A block holds one statement, or several separated by `;` at a line end — each
is run on its own, as the editor would. A line `:param name => value` sets a
parameter, as it does in the Studio. Options: rows=N (assert the row count of
the last statement), graph=NAME, shot=NAME (photograph this result in the
browser; read-only statements only, since it is run a second time).

Usage:  python3 build.py [--no-shots]
        python3 build.py --check        # run as a regression test; writes nothing
Needs:  the og-fabric container with the fabric-tut sources, and the Studio
        running against the tutorial database (see the tutorial's "준비").
Env:    TUT_STUDIO  the Studio's address      (default http://127.0.0.1:7479)
        TUT_EXEC    a shell next to PostgreSQL (default "docker exec -i og-fabric")
        TUT_DB      database to (re)create     (default tut_fabric)
"""
import json, os, re, shlex, subprocess, sys, time, unicodedata, urllib.error, urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
SRC = HERE / "tutorial.src.md"
OUT = ROOT / "docs" / "tutorials" / "virtual-fabric-tutorial.md"
IMAGES = ROOT / "docs" / "tutorials" / "images"
SCRIPT = HERE / "virtual_fabric.cypher"

STUDIO = os.environ.get("TUT_STUDIO", "http://127.0.0.1:7479").rstrip("/")
EXEC = shlex.split(os.environ.get("TUT_EXEC", "docker exec -i og-fabric"))
DB = os.environ.get("TUT_DB", "tut_fabric")
PSQL = ["psql", "-X", "-q", "-h", "localhost", "-p", os.environ.get("TUT_PGPORT", "28816")]
GRAPH = "scm"

params = {}  # what `:param` has set so far, as the Studio page would hold it


def admin(sql, db="postgres"):
    p = subprocess.run(EXEC + PSQL + ["-d", db], input=sql, capture_output=True, text=True)
    return p.returncode, p.stderr


def studio(graph, query):
    """One statement, as the Studio's editor sends it."""
    body = json.dumps({"graph": graph, "query": query, "params": params}).encode()
    req = urllib.request.Request(STUDIO + "/api/cypher", data=body, headers={"content-type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=120) as r:
            return True, json.load(r)
    except urllib.error.HTTPError as e:
        return False, json.load(e)


def statements(body):
    """Split a block where a line ends in `;`. Comment lines belong to nobody."""
    out, cur = [], []
    for line in body.splitlines():
        if not cur and (not line.strip() or line.strip().startswith("//")):
            continue
        cur.append(line)
        if line.rstrip().endswith(";"):
            out.append("\n".join(cur).rstrip().rstrip(";"))
            cur = []
    if "\n".join(cur).strip():
        out.append("\n".join(cur).rstrip())
    return out


PARAM = re.compile(r"^:param\s+([A-Za-z_]\w*)\s*=>\s*(.+)$", re.S)


def set_param(stmt):
    name, value = PARAM.match(stmt).groups()
    value = value.strip()
    try:
        params[name] = json.loads(value)
    except ValueError:
        params[name] = re.sub(r"^'(.*)'$", r"\1", value, flags=re.S)
    return f"${name} set for this page — not saved, not shown"


def width(s):
    return sum(2 if unicodedata.east_asian_width(c) in "WF" else 1 for c in s)


def cell(v):
    if v is None:
        return "null"
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, (dict, list)):
        return json.dumps(v, ensure_ascii=False)
    return str(v)


def table(cols, rows):
    cells = [[cell(r.get(c)) for c in cols] for r in rows]
    ws = [max([width(c)] + [width(r[i]) for r in cells]) for i, c in enumerate(cols)]
    line = lambda vals: (" " + " | ".join(v + " " * (w - width(v)) for v, w in zip(vals, ws))).rstrip()
    out = [line(cols), "-" + "-+-".join("-" * w for w in ws) + "-"] + [line(r) for r in cells]
    out.append(f"({len(rows)} row{'s' if len(rows) != 1 else ''})")
    return "\n".join(out)


def render(stmt, data):
    cols, rows = data.get("columns") or [], data.get("rows") or []
    if cols == ["result"]:
        return "\n".join(cell(r.get("result")) for r in rows)
    if re.match(r"\s*EXPLAIN\s+FABRIC\b", stmt, re.I):
        # The detail column is one long SQL text; two lines a step reads better.
        lines = []
        for r in rows:
            n = r["rows"]
            lines.append(f"{r['step']}. [{r['place']}] {r['operation']}  → {n} row{'' if n == 1 else 's'}")
            lines.append(f"   {r['detail']}")
        return "\n".join(lines)
    if not cols and not rows:
        return "(no rows)"
    return table(cols, rows)


FENCE = re.compile(r"^```(\w+)[ \t]+(run|error|sql)\b(.*)$")


def options(rest):
    opts = {"silent": "silent" in rest.split()}
    for key in ("rows", "graph", "shot"):
        m = re.search(rf"{key}=(\S+)", rest)
        opts[key] = m.group(1) if m else None
    opts["rows"] = int(opts["rows"]) if opts["rows"] else None
    opts["graph"] = opts["graph"] or GRAPH
    return opts


def main():
    shots_wanted = "--no-shots" not in sys.argv
    rc, err = admin(f"DROP DATABASE IF EXISTS {DB} WITH (FORCE);\nCREATE DATABASE {DB};")
    if rc != 0:
        sys.exit(f"cannot reach PostgreSQL through {' '.join(EXEC)}: {err}")
    rc, err = admin("CREATE EXTENSION ontological CASCADE;", db=DB)
    if rc != 0:
        sys.exit(f"cannot install the extension: {err}")
    # The Studio's pooled connections went with the old database; the first
    # requests after that may find a dead one.
    for _ in range(10):
        try:
            ok, _data = studio("default", "SHOW GRAPHS")
            if ok:
                break
        except OSError:
            pass
        time.sleep(1)
    else:
        sys.exit(f"the Studio at {STUDIO} is not answering — is it running against {DB}?")

    src = SRC.read_text().splitlines()
    out, script, shots, failures = [], [], [], []
    i = n_blocks = n_stmts = 0
    while i < len(src):
        m = FENCE.match(src[i])
        if not m:
            out.append(src[i])
            i += 1
            continue
        lang, mode, rest = m.groups()
        opts = options(rest)
        j = i + 1
        while src[j].rstrip() != "```":
            j += 1
        body = "\n".join(src[i + 1:j])
        where = f"block at line {i + 1}"
        i = j + 1
        n_blocks += 1
        shown, got_rows = [], None

        if lang == "cypher":
            for stmt in statements(body):
                n_stmts += 1
                if PARAM.match(stmt):
                    shown.append(set_param(stmt))
                    script.append("// " + re.sub(r"=>.*$", "=> …", stmt, flags=re.S))
                    continue
                ok, data = studio(opts["graph"], stmt)
                if mode == "error":
                    if ok:
                        failures.append(f"{where}: expected an error, got {len(data.get('rows', []))} rows\n{stmt}")
                    else:
                        shown.append("오류: " + data.get("error", ""))
                    script.append("// expected to fail:\n// " + stmt.replace("\n", "\n// "))
                    continue
                if not ok:
                    failures.append(f"{where}: {data.get('error')}\n{stmt}")
                    continue
                script.append(stmt + ";")
                got_rows = len(data.get("rows") or [])
                if mode == "sql":
                    shown.append(data.get("compiled_sql") or "(no single SQL statement)")
                else:
                    shown.append(render(stmt, data))
                if opts["shot"]:
                    shots.append((opts["shot"], opts["graph"], stmt))
        elif lang == "python":
            p = subprocess.run(EXEC + ["/tmp/nv/bin/python", "-"], input=body + "\n", capture_output=True, text=True)
            if p.returncode != 0:
                failures.append(f"{where}: {p.stderr.strip()[-400:]}")
            shown.append(p.stdout.rstrip())
        else:
            sys.exit(f"{where}: don't know how to run '{lang}'")

        if opts["rows"] is not None and got_rows != opts["rows"]:
            failures.append(f"{where}: expected {opts['rows']} rows, got {got_rows}")

        out += [f"```{lang}", body, "```"]
        text = "\n\n".join(s for s in shown if s)
        if text and not opts["silent"]:
            out += ["", "```text", text, "```"]
        if opts["shot"]:
            out += ["", f"![Studio — {opts['shot']}](images/virtual-fabric-{opts['shot']}.png)"]

    if failures:
        print(f"{len(failures)} failure(s) in {n_blocks} blocks:\n")
        for f in failures:
            print(" ✗", f, "\n")
        sys.exit(1)

    if "--check" in sys.argv:
        # Run as a test: every statement executed and asserted, nothing written.
        print(f"virtual-fabric: ok — {n_stmts} statements in {n_blocks} blocks ran through the Studio")
        return

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text("\n".join(out) + "\n")
    SCRIPT.write_text(
        "// Every statement of docs/tutorials/virtual-fabric-tutorial.md, in order.\n"
        "// Generated by build.py — edit tutorial.src.md instead.\n"
        "// Each is one entry in the Studio's editor (⌘/Ctrl + Enter), or one driver call over Bolt.\n\n"
        + "\n\n".join(script) + "\n")
    print(f"ok — {n_stmts} statements in {n_blocks} blocks ran through the Studio; wrote {OUT.relative_to(ROOT)}")

    if shots and shots_wanted:
        take_shots(shots)


def take_shots(shots):
    """Replay the marked statements in the Studio's page and photograph the result frame."""
    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        print("playwright is not installed — screenshots skipped")
        return
    IMAGES.mkdir(parents=True, exist_ok=True)
    with sync_playwright() as p:
        browser = p.chromium.launch()
        page = browser.new_page(viewport={"width": 1280, "height": 800}, device_scale_factor=2)
        problems = []
        page.on("pageerror", lambda e: problems.append(str(e)))
        page.goto(STUDIO + "/")
        page.wait_for_selector("#graph-select option", state="attached")
        for name, value in params.items():
            page.fill("#editor", f":param {name} => {json.dumps(value)}")
            page.keyboard.press("Control+Enter")
        for name, graph, stmt in shots:
            page.select_option("#graph-select", graph)
            page.fill("#editor", ":clear")
            page.keyboard.press("Control+Enter")
            page.fill("#editor", stmt)
            page.keyboard.press("Control+Enter")
            page.wait_for_selector(".frame .tab")
            page.wait_for_timeout(1500)  # let a force-directed graph settle
            page.query_selector(".frame").screenshot(path=str(IMAGES / f"virtual-fabric-{name}.png"))
        browser.close()
    if problems:
        sys.exit("the Studio page raised errors while replaying: " + "; ".join(problems))
    print(f"   {len(shots)} screenshots in {IMAGES.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
