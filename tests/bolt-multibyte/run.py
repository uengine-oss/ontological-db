#!/usr/bin/env python3
"""Queries whose first bytes hold a multibyte character, through the Bolt gateway.

The gateway looks at the start of every query for `EXPLAIN` / `PROFILE` (and
`EXPLAIN FABRIC`). It used to compare by slicing the text at byte 7 (and 6),
which is not a character boundary when a multibyte character straddles it —
a query opening with a Korean comment (`// 소스 …`), a Korean key or an emoji
panicked the session thread and the client saw the connection drop. This
sends such queries and expects ordinary answers, on the same session.

    python3 tests/bolt-multibyte/run.py   # needs psql, cargo, the neo4j driver

OG_BOLT_BIN runs a gateway binary that is already built instead of `cargo run`.
Exit status is non-zero on the first failed check.
"""

import os
import socket
import subprocess
import sys
import time

from neo4j import GraphDatabase

HOST = os.environ.get("PGHOST", "localhost")
PORT = os.environ.get("PGPORT", "28816")
USER = os.environ.get("PGUSER")
DB = "og_bolt_multibyte"
LISTEN = os.environ.get("OG_BOLT_TEST_LISTEN", "127.0.0.1:27693")
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def psql(sql, db=DB):
    cmd = ["psql", "-h", HOST, "-p", PORT, "-d", db, "-tAqX", "-v", "ON_ERROR_STOP=1", "-c", sql]
    if USER:
        cmd[1:1] = ["-U", USER]
    out = subprocess.run(cmd, capture_output=True, text=True)
    if out.returncode != 0:
        sys.exit(f"psql failed: {sql}\n{out.stderr}")
    return out.stdout.strip()


def check(what, ok, detail=""):
    if not ok:
        sys.exit(f"FAIL {what} {detail}")
    print(f"ok   {what}")


def gateway():
    env = dict(os.environ, OG_BOLT_LISTEN=LISTEN, OG_BOLT_PGHOST=HOST, OG_BOLT_PGPORT=PORT,
               OG_BOLT_PGDATABASE=DB, OG_BOLT_GRAPH="mb")
    cmd = [os.environ["OG_BOLT_BIN"]] if os.environ.get("OG_BOLT_BIN") else \
        ["cargo", "run", "-q", "--release", "--manifest-path", os.path.join(ROOT, "bolt", "Cargo.toml")]
    gw = subprocess.Popen(cmd, env=env)
    host, port = LISTEN.split(":")
    for _ in range(600):
        try:
            socket.create_connection((host, int(port)), 0.5).close()
            return gw
        except OSError:
            time.sleep(0.5)
    gw.kill()
    sys.exit("the gateway did not start")


def answer(s, q):
    """(rows, None) or (None, error code) — anything else (a dropped connection) is raised."""
    try:
        return [r.data() for r in s.run(q)], None
    except Exception as e:  # noqa: BLE001
        code = getattr(e, "code", None)
        if not code or not str(code).startswith("Neo.ClientError"):
            raise
        return None, code


def main():
    psql(f"DROP DATABASE IF EXISTS {DB}", db="postgres")
    psql(f"CREATE DATABASE {DB}", db="postgres")
    psql("CREATE EXTENSION ontological CASCADE")
    psql("SELECT og_create_graph('mb')")
    psql("SELECT og_cypher('mb', 'UNWIND [1, 2, 3] AS i CREATE (:Thing {n: i, label: ''가'' + toString(i)})')")
    user = psql("SELECT current_user")
    gw = gateway()
    try:
        driver = GraphDatabase.driver(f"bolt://{LISTEN}", auth=(user, ""))
        with driver.session() as s:
            rows, err = answer(s, "// 소스 확인: 물건 수\nMATCH (n:Thing) RETURN count(n) AS n")
            check("1  a query opening with a Korean comment is answered", rows == [{"n": 3}], (rows, err))
            rows, err = answer(s, "//🙂🙂 smile\nRETURN 1 AS x")
            check("2  a query opening with an emoji comment is answered", rows == [{"x": 1}], (rows, err))
            rows, err = answer(s, "WITH '🙂' AS face RETURN face")
            check("3  an emoji literal in the first bytes", rows == [{"face": "🙂"}], (rows, err))
            rows, err = answer(s, "WITH '가나' AS w MATCH (n:Thing) WHERE n.label = '가1' RETURN w, n.n AS n")
            check("4  a Korean literal in the first bytes", rows == [{"w": "가나", "n": 1}], (rows, err))
            rows, err = answer(s, "WITH {수량: 3} AS m RETURN m.수량 AS v")
            check("5  a Korean key in the first bytes", rows == [{"v": 3}], (rows, err))
            rows, err = answer(s, "가나다")
            check("6  Korean text alone is a syntax error, not a dropped connection",
                  rows is None and "Syntax" in err, (rows, err))
            rows, err = answer(s, "EXPLAIN //소스\nMATCH (n:Thing) RETURN n")
            check("7  EXPLAIN followed by a Korean comment plans without records", rows == [], (rows, err))
            rows, err = answer(s, "PROFILE //🙂\nMATCH (n:Thing) RETURN n")
            check("8  PROFILE followed by an emoji comment plans without records", rows == [], (rows, err))
            rows, err = answer(s, "RETURN 1 AS x")
            check("9  the same session still answers", rows == [{"x": 1}], (rows, err))
        driver.close()
    finally:
        gw.terminate()
        gw.wait(10)
    print("bolt multibyte: all checks passed")


if __name__ == "__main__":
    main()
