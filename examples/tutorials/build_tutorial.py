#!/usr/bin/env python3
"""Build a tutorial by running it:  examples/tutorials/<name>/tutorial.src.md
→ docs/tutorials/<name>-tutorial.md, with real output under every example.

    python3 examples/tutorials/build_tutorial.py <name>            # build one
    python3 examples/tutorials/build_tutorial.py --check --all     # run every tutorial as a test

`--check` writes nothing: it runs every example and fails if one errors, if an
error that was expected does not happen, or if a `rows=` / `expect=` assertion
does not hold. That makes each tutorial a regression test — `tests/tutorials.sh`
runs them all before a release. Output that merely differs from the published
page (a timestamp, an id) is listed as drift and fails only with `--strict`.

The source is the tutorial as Markdown. A fenced block whose info string has a
directive is executed, in order, against a database created empty for this run
(`tut_<name>`, with the extension installed), and what it printed is written
beneath it. Nothing in a built tutorial's output is typed by hand; if an example
stops working, the build fails and says which.

    ```sql run              run with psql, show the output
    ```sql run silent       run, show nothing
    ```sql error            must fail; show the error
    ```cypher run           run through og_cypher(graph, …); a table in RETURN order
    ```cypher run silent
    ```cypher error         must fail; show the error

Options after the directive:  graph=NAME (cypher; default: the tutorial's GRAPH
line, see below) · rows=N (assert the row count of the last result) ·
expect="text" (assert the output contains it; may be repeated) ·
params={"k": v} (cypher parameters).

The first line of the source may be an HTML comment naming the default graph:
    <!-- graph: shop -->

Env: TUT_EXEC  a shell next to PostgreSQL (default "docker exec -i og-fabric")
     TUT_PGPORT  (default 28816)
"""
import json, os, re, shlex, subprocess, sys, unicodedata
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
EXEC = shlex.split(os.environ.get("TUT_EXEC", "docker exec -i og-fabric"))
PSQL = ["psql", "-X", "-q", "-h", "localhost", "-p", os.environ.get("TUT_PGPORT", "28816")]


def psql(sql, db, *, tuples=False, stop=True):
    args = PSQL + ["-d", db] + (["-At"] if tuples else []) + (["-v", "ON_ERROR_STOP=1"] if stop else [])
    p = subprocess.run(EXEC + args, input=sql, capture_output=True, text=True)
    return p.returncode, p.stdout, p.stderr


def clean(text):
    keep = [l.rstrip() for l in text.splitlines() if not l.startswith(("NOTICE:", "psql:"))]
    while keep and not keep[-1]:
        keep.pop()
    while keep and not keep[0]:
        keep.pop(0)
    return "\n".join(keep)


def error_of(stderr):
    return "\n".join(l for l in stderr.splitlines() if l.startswith(("ERROR:", "DETAIL:", "HINT:")))


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
    return "\n".join([line(cols), "-" + "-+-".join("-" * w for w in ws) + "-"] + [line(r) for r in cells]
                     + [f"({len(rows)} row{'s' if len(rows) != 1 else ''})"])


def cypher(q, graph, params, db):
    p = f", $p${json.dumps(params, ensure_ascii=False)}$p$::jsonb" if params else ""
    sql = (f"SELECT array_to_json(og_cypher_columns($cy${q}$cy$));\n"
           f"SELECT coalesce(jsonb_agg(r), '[]') FROM og_cypher('{graph}', $cy${q}$cy${p}) r;")
    rc, out, err = psql(sql, db, tuples=True)
    if rc != 0:
        return None, error_of(err) or err.strip()
    lines = out.strip().splitlines()
    cols, rows = json.loads(lines[0]), json.loads(lines[1])
    if not cols and rows:
        cols = list(rows[0].keys())
    return (cols, rows), None


FENCE = re.compile(r"^```(sql|cypher)[ \t]+(run|error)\b(.*)$")
# A source this builder runs, as opposed to one another builder owns.
FENCE_ANY = re.compile(r"^```(?:sql|cypher)[ \t]+(?:run|error)\b", re.M)


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    flags = {a for a in sys.argv[1:] if a.startswith("--")}
    check, strict = "--check" in flags or "--strict" in flags, "--strict" in flags
    if "--all" in flags:
        # A directory with its own build.py is built by that (the fabric tutorial runs through the Studio).
        args = sorted(p.parent.name for p in HERE.glob("*/tutorial.src.md")
                      if FENCE_ANY.search(p.read_text()) and not (p.parent / "build.py").exists())
    if not args:
        sys.exit(__doc__)
    results = [build(name, check, strict) for name in args]
    if check:
        print(f"\n{sum(results)} of {len(results)} tutorials passed")
    sys.exit(0 if all(results) else 1)


def build(name, check=False, strict=False):
    src_path = HERE / name / "tutorial.src.md"
    out_path = ROOT / "docs" / "tutorials" / f"{name}-tutorial.md"
    # One database per tutorial, for a build and a check alike: a tutorial may create
    # cluster-wide roles, and those can only be dropped once the database holding
    # their grants has been — which recreating it here does.
    db = "tut_" + re.sub(r"\W", "_", name)
    src = src_path.read_text().splitlines()
    default_graph = "default"
    if src and (m := re.match(r"<!--\s*graph:\s*(\w+)\s*-->", src[0])):
        default_graph, src = m.group(1), src[1:]

    rc, _, err = psql(f"DROP DATABASE IF EXISTS {db} WITH (FORCE);\nCREATE DATABASE {db};", "postgres", stop=False)
    if rc != 0:
        sys.exit(f"cannot reach PostgreSQL through {' '.join(EXEC)}: {err}")
    rc, _, err = psql("CREATE EXTENSION ontological CASCADE;", db)
    if rc != 0:
        sys.exit(f"cannot install the extension: {err}")
    if check:
        # A tutorial run as a test is run where no reader will look for it.
        db_note = f"{name}: "

    out, failures, i, n = [], [], 0, 0
    while i < len(src):
        m = FENCE.match(src[i])
        if not m:
            out.append(src[i])
            i += 1
            continue
        lang, mode, rest = m.groups()
        silent = "silent" in rest.split()
        want_rows = int(x.group(1)) if (x := re.search(r"rows=(\d+)", rest)) else None
        graph = x.group(1) if (x := re.search(r"graph=(\w+)", rest)) else default_graph
        params = json.loads(x.group(1)) if (x := re.search(r"params=(\{.*\})", rest)) else None
        expects = re.findall(r'expect="([^"]*)"', rest)
        j = i + 1
        while src[j].rstrip() != "```":
            j += 1
        body = "\n".join(src[i + 1:j])
        where = f"block at source line {i + 1}"
        i, n = j + 1, n + 1
        shown, got = "", None

        if lang == "sql":
            rc, o, e = psql(body + "\n", db, stop=(mode != "error"))
            if mode == "error":
                shown = error_of(e)
                if not shown:
                    failures.append(f"{where}: expected an error, got none")
            else:
                if rc != 0:
                    failures.append(f"{where}: {error_of(e) or e.strip()}")
                shown = clean(o)
                found = re.findall(r"^\((\d+) rows?\)$", shown, re.M)
                got = int(found[-1]) if found else None
        else:
            res, e = cypher(body, graph, params, db)
            if mode == "error":
                shown = e or ""
                if not e:
                    failures.append(f"{where}: expected an error, got none")
            elif e:
                failures.append(f"{where}: {e}")
            else:
                cols, rows = res
                got = len(rows)
                shown = table(cols, rows) if (cols or rows) else "(no rows)"
        if want_rows is not None and got != want_rows:
            failures.append(f"{where}: expected {want_rows} rows, got {got}\n{shown}")
        for want in expects:
            if want not in shown:
                failures.append(f"{where}: the output does not contain \"{want}\"\n{shown}")
        out += [f"```{lang}", body, "```"]
        if shown and not silent:
            out += ["", "```text", shown, "```"]

    if failures:
        print(f"{name}: {len(failures)} of {n} blocks failed:\n")
        for f in failures:
            print(" ✗", f, "\n")
        if not check:
            sys.exit(1)
        return False
    built = "\n".join(out) + "\n"
    if check:
        published = out_path.read_text() if out_path.exists() else ""
        if built == published:
            print(f"{db_note}ok — {n} blocks, output identical to the published page")
            return True
        a, b = published.splitlines(), built.splitlines()
        drift = sum(1 for x, y in zip(a, b) if x != y) + abs(len(a) - len(b))
        print(f"{db_note}{'FAILED' if strict else 'ok'} — {n} blocks ran, but {drift} lines differ from "
              f"{out_path.relative_to(ROOT)} (rebuild it, or the output is not deterministic)")
        return not strict
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(built)
    print(f"ok — {n} blocks ran in {db}; wrote {out_path.relative_to(ROOT)}")
    return True


if __name__ == "__main__":
    main()
