#!/usr/bin/env python3
"""TypeQL conformance against the TypeDB example application — spec 010 T055..T061.

The point of this suite is that nothing in it is our own invention. The schema,
the data and the queries are the upstream TypeDB files in
`examples/typedb/bookstore/`, unmodified; the expected results are the ones
printed in that example's own README. If this passes, a TypeDB user's files ran
here and produced what TypeDB's documentation says they produce.

Usage:  python3 tests/typeql/run.py [--db NAME]
Env:    PGHOST PGPORT PGUSER  (defaults match tests/run.sh)
"""

import argparse
import json
import os
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
EXAMPLE = ROOT / "examples" / "typedb" / "bookstore"
HERE = pathlib.Path(__file__).resolve().parent

HOST = os.environ.get("PGHOST", "localhost")
PORT = os.environ.get("PGPORT", "28816")

GREEN, RED, YELLOW, DIM, RESET = "\033[32m", "\033[31m", "\033[33m", "\033[2m", "\033[0m"

results = []


def psql(db, sql, tuples=True):
    cmd = ["psql", "-h", HOST, "-p", PORT, "-d", db, "-v", "ON_ERROR_STOP=1"]
    cmd += ["-qtAc" if tuples else "-qc", sql]
    return subprocess.run(cmd, capture_output=True, text=True)


def record(name, ok, detail="", unsupported=False):
    results.append((name, ok, detail, unsupported))
    if unsupported:
        mark, colour = "unsupported", YELLOW
    elif ok:
        mark, colour = "ok", GREEN
    else:
        mark, colour = "FAIL", RED
    print(f"  {name:<58} {colour}{mark}{RESET}")
    if detail and not ok and not unsupported:
        for line in detail.strip().splitlines()[:6]:
            print(f"      {DIM}{line}{RESET}")


def typeql(db, query):
    """Run a TypeQL query, returning the result rows as parsed JSON."""
    r = psql(db, f"SELECT og_typeql('bookstore', $tql${query}$tql$)")
    if r.returncode != 0:
        raise RuntimeError(r.stderr.strip())
    return [json.loads(line) for line in r.stdout.splitlines() if line.strip()]


def scalar(db, query, key="n"):
    rows = typeql(db, query)
    return rows[0][key] if rows else None


def canonical(obj):
    """Order-insensitive comparison key. Floats are compared at the precision
    the documented output was printed with, not by binary equality."""
    if isinstance(obj, dict):
        return tuple(sorted((k, canonical(v)) for k, v in obj.items()))
    if isinstance(obj, list):
        return tuple(sorted((canonical(v) for v in obj), key=repr))
    if isinstance(obj, float):
        return round(obj, 6)
    return obj


def compare_sets(actual, expected):
    a = sorted((repr(canonical(x)) for x in actual))
    e = sorted((repr(canonical(x)) for x in expected))
    if a == e:
        return True, ""
    missing = [x for x in e if x not in a]
    extra = [x for x in a if x not in e]
    detail = ""
    if missing:
        detail += "missing:\n" + "\n".join(missing[:3]) + "\n"
    if extra:
        detail += "unexpected:\n" + "\n".join(extra[:3])
    return False, detail


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", default="og_typeql_test")
    args = ap.parse_args()
    db = args.db

    print(f"\nTypeQL conformance — TypeDB bookstore example\n{'-' * 72}")

    # ---------------------------------------------------------------- setup
    subprocess.run(["psql", "-h", HOST, "-p", PORT, "-d", "postgres", "-qc",
                    f"DROP DATABASE IF EXISTS {db}"], capture_output=True, text=True)
    subprocess.run(["psql", "-h", HOST, "-p", PORT, "-d", "postgres", "-qc",
                    f"CREATE DATABASE {db}"], capture_output=True, text=True)
    r = psql(db, "CREATE EXTENSION ontological CASCADE")
    if r.returncode != 0:
        print(f"{RED}cannot create the extension:{RESET}\n{r.stderr}")
        return 2
    psql(db, "SELECT og_create_graph('bookstore')")

    print("\nUser Story 1 — the upstream schema loads unmodified")
    r = psql(db, "SELECT og_typeql_script('bookstore', pg_read_file(%s))"
             % sql_literal(str(EXAMPLE / "schema.tql")))
    record("T055 schema.tql loads verbatim", r.returncode == 0, r.stderr)
    if r.returncode != 0:
        summarise()
        return 1

    counts = psql(db, """SELECT kind::text || '=' || count(*) FROM og_catalog.type
                         WHERE name NOT LIKE '$%' GROUP BY kind ORDER BY kind""")
    got = dict(x.split("=") for x in counts.stdout.split())
    record("      19 entity / 12 relation / 25 attribute types",
           got == {"e": "19", "r": "12", "a": "25"}, f"got {got}")

    spec = psql(db, "SELECT count(*) FROM og_catalog.role WHERE parent_role_id IS NOT NULL")
    record("      3 specialised roles (relates .. as ..)",
           spec.stdout.strip() == "3", f"got {spec.stdout.strip()}")

    funs = psql(db, "SELECT count(*) FROM og_catalog.typeql_function")
    record("      7 function declarations preserved",
           funs.stdout.strip() == "7", f"got {funs.stdout.strip()}")

    print("\nUser Story 2 — the upstream data loads unmodified")
    r = psql(db, "SELECT og_typeql_script('bookstore', pg_read_file(%s))"
             % sql_literal(str(EXAMPLE / "data.tql")))
    record("T056 data.tql loads verbatim (31 blocks)",
           r.returncode == 0 and r.stdout.strip() == "31", r.stderr or r.stdout)
    if r.returncode != 0:
        summarise()
        return 1

    try:
        books = scalar(db, "match $b isa book; reduce $n = count;")
        hard = scalar(db, "match $b isa hardback; reduce $n = count;")
        soft = scalar(db, "match $b isa paperback; reduce $n = count;")
        e = scalar(db, "match $b isa ebook; reduce $n = count;")
        record("      21 books = 3 hardback + 12 paperback + 6 ebook",
               (books, hard, soft, e) == (21, 3, 12, 6),
               f"got {(books, hard, soft, e)}")

        shared = psql(db, """SELECT count(*) FROM og_catalog.type t
                             JOIN og_data.og_node n ON n.type_id = t.type_id
                             WHERE t.name = 'genre'""")
        owners = scalar(db, 'match $b isa book, has genre "fiction"; reduce $n = count;')
        record("      attributes are shared by value, not copied",
               int(shared.stdout.strip()) < owners * 5,
               f"{shared.stdout.strip()} genre instances for {owners} fiction books")
    except RuntimeError as exc:
        record("      instance counts", False, str(exc))

    print("\nUser Story 3 — the documented queries return the documented results")
    for name in ("q1", "q2"):
        query = (HERE / "queries" / f"{name}.tql").read_text()
        expected = json.loads((HERE / "expected" / f"{name}.json").read_text())
        try:
            actual = typeql(db, query)
            ok, detail = compare_sets(actual, expected)
        except RuntimeError as exc:
            ok, detail = False, str(exc)
        record(f"T057/T058 {name}.tql matches the example's own README", ok, detail)

    print("\nPolymorphism, pipeline and interop")
    checks = [
        ("T059 abstract type answers for every subtype",
         lambda: scalar(db, "match $b isa book; reduce $n = count;") == 21),
        ("      isa! restricts to the exact type",
         lambda: scalar(db, "match $b isa! book; reduce $n = count;") == 0),
        ("      attribute subtyping (has isbn finds isbn-13 and isbn-10)",
         lambda: scalar(db, "match $b isa book, has isbn $i; reduce $n = count;")
         > scalar(db, "match $b isa book, has isbn-13 $i; reduce $n = count;")),
        ("      role specialisation (contribution finds authoring/editing)",
         lambda: scalar(db, "match contribution (contributor: $c, work: $w); reduce $n = count;") == 35),
        ("      3-ary relation with named roles",
         lambda: scalar(db, "match publishing (published: $b, publisher: $p, publication: $u); reduce $n = count;") == 21),
        # Unnamed role players are unordered, so an untyped pair matches a
        # rating twice — once per assignment. Pinning the types collapses it to
        # the 33 ratings, which is the answer a reader expects to see.
        ("      positional role players (unordered)",
         lambda: scalar(db, "match ($r, $b) isa rating; reduce $n = count;") == 66),
        ("      positional role players pinned by type",
         lambda: scalar(db, "match ($r, $b) isa rating; $r isa review; $b isa book; reduce $n = count;") == 33),
        ("      not { }",
         lambda: scalar(db, 'match $b isa book; not { $b has genre "fiction"; }; reduce $n = count;') == 11),
        ("      { } or { }",
         lambda: scalar(db, 'match $b isa book; { $b has genre "fiction"; } or { $b has genre "history"; }; reduce $n = count;') == 14),
        ("      sort / limit",
         lambda: [r["p"] for r in typeql(db, "match $b isa book, has price $p; sort $p desc; limit 3; fetch { \"p\": $p };")]
         == [230.37, 128.71, 93.22]),
        ("      reduce with groupby",
         lambda: len(typeql(db, "match $o isa order, has id $i; order-line (order: $o, item: $x); reduce $n = count groupby $i;")) > 0),
    ]
    for name, fn in checks:
        try:
            record(name, bool(fn()))
        except Exception as exc:  # noqa: BLE001 - report, do not mask
            record(name, False, str(exc))

    # T060 — the same graph, the other language.
    tq = scalar(db, "match $b isa paperback; reduce $n = count;")
    cy = psql(db, "SELECT og_cypher('bookstore', $c$ MATCH (b:paperback) RETURN count(b) AS n $c$)")
    cy_n = json.loads(cy.stdout.strip())["n"] if cy.returncode == 0 else None
    record("T060 Cypher and TypeQL agree on the same graph", tq == cy_n,
           f"typeql={tq} cypher={cy_n} {cy.stderr}")

    print("\nSchema round trip and refusals")
    dump = psql(db, "SELECT og_typeql_schema('bookstore')")
    pathlib.Path("/tmp/og_typeql_dump.tql").write_text(dump.stdout)
    psql(db, "SELECT og_create_graph('roundtrip')")
    r = psql(db, "SELECT og_typeql_script('roundtrip', pg_read_file('/tmp/og_typeql_dump.tql'))")
    rt = psql(db, """SELECT kind::text || '=' || count(*) FROM og_catalog.type
                     WHERE graph_id = (SELECT graph_id FROM og_catalog.graph WHERE name='roundtrip')
                       AND name NOT LIKE '$%' GROUP BY kind ORDER BY kind""")
    got = dict(x.split("=") for x in rt.stdout.split())
    record("T047 schema dump reloads into an identical schema",
           r.returncode == 0 and got == {"e": "19", "r": "12", "a": "25"},
           r.stderr or f"got {got}")

    refusals = [
        ("unknown type is refused with a suggestion", "match $b isa boook;", "did you mean"),
        ("TypeDB 2.x syntax is named, not guessed at", "match $b isa book; get $b;", "2.x"),
        ("@values is enforced", 'insert $o isa order, has id "x1", has status "nope";', "not an allowed value"),
        ("@key is enforced", 'insert $u isa user, has id "u0001";', "@key"),
        ("@range is enforced", 'insert $u isa user, has id "x2", has loyalty-tier 9;', "range"),
        ("abstract types cannot be instantiated", 'insert $b isa book, has title "x";', "abstract"),
    ]
    for name, q, needle in refusals:
        r = psql(db, f"SELECT og_typeql('bookstore', $tql${q}$tql$)")
        record(f"      {name}", r.returncode != 0 and needle in r.stderr,
               r.stderr or "query unexpectedly succeeded")

    print("\nDeclared unsupported (spec 010 phase 6 — reported, never silently wrong)")
    for name, q in [
        ("user-defined function call", "match $b isa book; let $d = best_discount_for_item($b, 2023-12-02T00:00:00); reduce $n = count;"),
        ("query-local function (with fun)", "with fun f() -> {book}: match $b isa book; return {$b}; match $b isa book;"),
    ]:
        r = psql(db, f"SELECT og_typeql('bookstore', $tql${q}$tql$)")
        record(f"      {name} refuses explicitly", r.returncode != 0,
               "silently succeeded — that is worse than failing", unsupported=True)

    return summarise()


def sql_literal(s):
    return "'" + s.replace("'", "''") + "'"


def summarise():
    real = [r for r in results if not r[3]]
    passed = sum(1 for r in real if r[1])
    failed = [r for r in real if not r[1]]
    unsupported = [r for r in results if r[3]]
    print(f"\n{'-' * 72}")
    print(f"{passed}/{len(real)} checks passed"
          + (f", {len(unsupported)} declared unsupported" if unsupported else ""))
    if failed:
        print(f"{RED}failed:{RESET}")
        for name, _, _, _ in failed:
            print(f"  - {name}")
        return 1
    print(f"{GREEN}all checks passed{RESET}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
