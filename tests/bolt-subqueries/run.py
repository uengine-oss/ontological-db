#!/usr/bin/env python3
"""CALL { … } subqueries, the functions added beside them, and what a session
on one database can reach of another — through the Bolt gateway, as a driver
sends them.

  * CALL { … } runs once per row: reading ones as a correlated join, writing
    ones row by row; OPTIONAL CALL keeps a row the body has nothing for;
    IN TRANSACTIONS is refused as a syntax error and the session goes on.
  * range(), left(), right(), reverse(), sign(), nullIf(), isEmpty(), toBoolean().
  * Two databases (graphs) behind one gateway: a map parameter with another
    graph's `_id` reaches nothing — not with MATCH, not with SET or DELETE —
    and `MATCH (n)` counts only its own graph. `id(n) = $id` takes the number a
    driver sends; keys(n) holds properties only, and a user property named `id`
    is a property of the Node the driver builds.
  * Temporal parameters the params suite does not send: a datetime in a named
    zone, a time with an offset, a negative duration, a date before 1970.

    python3 tests/bolt-subqueries/run.py   # needs psql, cargo, the neo4j driver

OG_BOLT_BIN runs a gateway binary that is already built instead of `cargo run`.
Exit status is non-zero on the first failed check.
"""

import datetime
import os
import socket
import subprocess
import sys
import time

from neo4j import GraphDatabase
from neo4j.time import Duration, Time

HOST = os.environ.get("PGHOST", "localhost")
PORT = os.environ.get("PGPORT", "28816")
USER = os.environ.get("PGUSER")
DB = "og_bolt_subqueries"
LISTEN = os.environ.get("OG_BOLT_TEST_LISTEN", "127.0.0.1:27697")
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
               OG_BOLT_PGDATABASE=DB, OG_BOLT_GRAPH="home")
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


def answer(s, q, **params):
    """(rows, None) or (None, (code, message)) — a dropped connection is raised."""
    try:
        return [r.data() for r in s.run(q, **params)], None
    except Exception as e:  # noqa: BLE001
        code = getattr(e, "code", None)
        if not code or not str(code).startswith("Neo.ClientError"):
            raise
        return None, (code, getattr(e, "message", None) or str(e))


def main():
    psql(f"DROP DATABASE IF EXISTS {DB}", db="postgres")
    psql(f"CREATE DATABASE {DB}", db="postgres")
    psql("CREATE EXTENSION ontological CASCADE")
    psql("SELECT og_create_graph('home')")
    psql("SELECT og_create_graph('away')")
    user = psql("SELECT current_user")
    gw = gateway()
    try:
        driver = GraphDatabase.driver(f"bolt://{LISTEN}", auth=(user, ""))
        with driver.session(database="home") as s:
            answer(s, "CREATE (a:Shelf {name: 'a'})-[:HOLDS]->(:Box {v: 1}), (a)-[:HOLDS]->(:Box {v: 2}), (:Shelf {name: 'b'})")

            # ------------------------------------------------------------ CALL { … }
            rows, err = answer(s, "MATCH (s:Shelf) CALL { WITH s MATCH (s)-[:HOLDS]->(b) RETURN count(b) AS n } "
                                  "RETURN s.name AS s, n ORDER BY s")
            check("1  a reading subquery, once per row, an aggregate over nothing is 0",
                  rows == [{"s": "a", "n": 2}, {"s": "b", "n": 0}], (rows, err))
            rows, err = answer(s, "MATCH (s:Shelf) CALL (s) { MATCH (s)-[:HOLDS]->(b) RETURN b.v AS v } "
                                  "RETURN s.name AS s, v ORDER BY v")
            check("2  rows multiply; a row the body has nothing for is dropped",
                  rows == [{"s": "a", "v": 1}, {"s": "a", "v": 2}], (rows, err))
            rows, err = answer(s, "MATCH (s:Shelf) OPTIONAL CALL (s) { MATCH (s)-[:HOLDS]->(b) RETURN b.v AS v } "
                                  "RETURN s.name AS s, v ORDER BY s, v")
            check("3  OPTIONAL CALL keeps it, with nulls",
                  rows == [{"s": "a", "v": 1}, {"s": "a", "v": 2}, {"s": "b", "v": None}], (rows, err))
            rows, err = answer(s, "UNWIND [1, 1, 1] AS i CALL { WITH i MERGE (t:Tally {k: i}) "
                                  "ON MATCH SET t.hits = coalesce(t.hits, 0) + 1 RETURN coalesce(t.hits, 0) AS h } RETURN collect(h) AS hs")
            check("4  a writing subquery runs row by row, each run seeing the last",
                  rows == [{"hs": [0, 1, 2]}], (rows, err))
            rows, err = answer(s, "MATCH (s:Shelf) CALL { WITH s CREATE (:Log {of: s.name}) } RETURN count(*) AS rows")
            check("5  a unit subquery leaves the rows as they were", rows == [{"rows": 2}], (rows, err))
            rows, err = answer(s, "UNWIND [1] AS i CALL { WITH i CREATE (:Never) } IN TRANSACTIONS OF 100 ROWS")
            check("6  IN TRANSACTIONS is refused as a syntax error",
                  rows is None and err[0] == "Neo.ClientError.Statement.SyntaxError" and "IN TRANSACTIONS" in err[1], (rows, err))
            rows, err = answer(s, "MATCH (n:Never) RETURN count(n) AS c")
            check("7  and wrote nothing; the session goes on", rows == [{"c": 0}], (rows, err))

            # ------------------------------------------------------------ functions
            rows, err = answer(s, "RETURN range(1, 4) AS r, range(6, 0, -3) AS r2, left($w, 2) AS l, right($w, 3) AS rt, "
                                  "reverse($w) AS rv, reverse([1, 2]) AS rl, sign(-3) AS sg, nullIf($w, 'hello') AS n, "
                                  "isEmpty($e) AS e, toBoolean('false') AS b", w="hello", e=[])
            check("8  range, left, right, reverse, sign, nullIf, isEmpty, toBoolean",
                  rows == [{"r": [1, 2, 3, 4], "r2": [6, 3, 0], "l": "he", "rt": "llo", "rv": "olleh", "rl": [2, 1],
                            "sg": -1, "n": None, "e": True, "b": False}], (rows, err))

            # ------------------------------------------------------------ id, keys, the Node a driver builds
            answer(s, "CREATE (:Lamp {id: 'singleton', watts: 40})")
            lamp = s.run("MATCH (n:Lamp) RETURN n, id(n) AS i, keys(n) AS k").single()
            check("9  a user property named id is a property of the Node; keys() holds properties only",
                  lamp["n"]["id"] == "singleton" and sorted(lamp["k"]) == ["id", "watts"] and lamp["n"].id == lamp["i"],
                  (dict(lamp["n"]), lamp["k"], lamp["n"].id, lamp["i"]))
            rows, err = answer(s, "MATCH (n) WHERE id(n) = $i RETURN n.watts AS w", i=lamp["i"])
            check("10 id(n) = $id takes the number a driver sends", rows == [{"w": 40}], (rows, err))
            rows, err = answer(s, "MATCH (n:Lamp {id: $id}) RETURN n.watts AS w", id="singleton")
            check("11 {id: $id} matches the property", rows == [{"w": 40}], (rows, err))

        # ------------------------------------------------------------ one database cannot reach another
        with driver.session(database="away") as a:
            far = a.run("CREATE (n:Far {name: 'f'}) RETURN id(n) AS i").single()["i"]
            rows, err = answer(a, "MATCH (n) RETURN count(n) AS c")
            check("12 MATCH (n) counts only its own graph", rows == [{"c": 1}], (rows, err))
        with driver.session(database="home") as s:
            rows, err = answer(s, "MATCH (n) WHERE id(n) = $i RETURN count(n) AS c", i=far)
            check("13 another graph's id matches nothing", rows == [{"c": 0}], (rows, err))
            rows, err = answer(s, "WITH $m AS n MATCH (n)-->(x) RETURN count(x) AS c", m={"_id": far})
            check("14 nor does a map that names it", rows == [{"c": 0}], (rows, err))
            rows, err = answer(s, "WITH $m AS n SET n.hacked = true", m={"_id": far})
            check("15 SET through such a map is refused, by what it is",
                  rows is None and "is not a node or relationship of this graph" in err[1], (rows, err))
            rows, err = answer(s, "WITH $m AS n DETACH DELETE n", m={"_id": far})
            check("16 so is DETACH DELETE", rows is None and "not a node or relationship of this graph" in err[1], (rows, err))
        with driver.session(database="away") as a:
            rows, err = answer(a, "MATCH (n:Far) RETURN count(n) AS c, collect(n.hacked) AS h")
            check("17 and the other graph is as it was", rows == [{"c": 1, "h": []}], (rows, err))

        # ------------------------------------------------------------ temporal parameters
        with driver.session(database="home") as s:
            # pytz zones: the 4.4 driver cannot encode a time whose tzinfo is a
            # stdlib datetime.timezone (a client-side TypeError), and sends a
            # pytz zone as a zone id — the structure this checks.
            import pytz
            seoul = pytz.timezone("Asia/Seoul").localize(datetime.datetime(2024, 3, 5, 13, 4, 5))
            rows, err = answer(s, "RETURN $zoned AS z, $t AS t, $neg AS d, $old AS o, $nano AS n, "
                                  "toString(datetime($zoned)) = toString(datetime('2024-03-05T04:04:05Z')) AS same_instant",
                               zoned=seoul, t=Time(9, 30, 15, tzinfo=pytz.FixedOffset(-210)), neg=Duration(days=-1, hours=-2),
                               old=datetime.date(1969, 7, 20), nano=datetime.datetime(2024, 1, 2, 3, 4, 5, 120000))
            check("18 a datetime in a named zone is that local time in that zone",
                  rows is not None and rows[0]["z"] == "2024-03-05T13:04:05[Asia/Seoul]" and rows[0]["same_instant"], (rows, err))
            check("19 a time with an offset, a negative duration, a date before 1970, a fraction of a second",
                  rows is not None and rows[0]["t"] == "09:30:15-03:30" and rows[0]["d"] == "P-1DT-2H"
                  and rows[0]["o"] == "1969-07-20" and rows[0]["n"] == "2024-01-02T03:04:05.12", (rows, err))
        driver.close()
    finally:
        gw.terminate()
        gw.wait(10)
    print("bolt subqueries: all checks passed")


if __name__ == "__main__":
    main()
