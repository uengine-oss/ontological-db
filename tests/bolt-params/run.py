#!/usr/bin/env python3
"""Typed parameters and path results, through the Bolt gateway, as a driver sends them.

A date, time, datetime or duration parameter reaches the gateway as a Bolt
structure. It used to be stored as the text `<unsupported struct 0x64>` — a
silent wrong answer. Now it is the ISO 8601 text Neo4j prints it as, which the
engine reads back as a date; a point (no form here) is refused by name. A path
returned to a driver is a Path, with its nodes and relationships.

    python3 tests/bolt-params/run.py   # needs psql, cargo, the neo4j driver

OG_BOLT_BIN runs a gateway binary that is already built instead of `cargo run`.
Exit status is non-zero on the first failed check.
"""

import os
import socket
import subprocess
import sys
import time

import datetime

from neo4j import GraphDatabase
from neo4j.spatial import CartesianPoint
from neo4j.time import Date, DateTime, Duration, Time

HOST = os.environ.get("PGHOST", "localhost")
PORT = os.environ.get("PGPORT", "28816")
USER = os.environ.get("PGUSER")
DB = "og_bolt_params"
LISTEN = os.environ.get("OG_BOLT_TEST_LISTEN", "127.0.0.1:27694")
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
               OG_BOLT_PGDATABASE=DB, OG_BOLT_GRAPH="pr")
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
    """(rows, None) or (None, error code) — anything else (a dropped connection) is raised."""
    try:
        return [r.data() for r in s.run(q, **params)], None
    except Exception as e:  # noqa: BLE001
        code = getattr(e, "code", None)
        if not code or not str(code).startswith("Neo.ClientError"):
            raise
        return None, code


def main():
    psql(f"DROP DATABASE IF EXISTS {DB}", db="postgres")
    psql(f"CREATE DATABASE {DB}", db="postgres")
    psql("CREATE EXTENSION ontological CASCADE")
    psql("SELECT og_create_graph('pr')")
    user = psql("SELECT current_user")
    gw = gateway()
    try:
        driver = GraphDatabase.driver(f"bolt://{LISTEN}", auth=(user, ""))
        with driver.session() as s:
            rows, err = answer(s, "RETURN $d AS d, $dt AS dt, $t AS t, $du AS du",
                               d=Date(2024, 3, 5), dt=DateTime(2024, 3, 5, 13, 4, 5), t=Time(13, 4, 5),
                               du=Duration(years=1, months=2, days=3, hours=4))
            check("1  date / datetime / time / duration parameters are not placeholders",
                  rows is not None and not any("unsupported" in str(v) for v in rows[0].values()), (rows, err))
            check("2  a date parameter is its ISO text", rows is not None and rows[0]["d"] == "2024-03-05", (rows, err))
            check("3  a datetime parameter is its ISO text", rows is not None and rows[0]["dt"] == "2024-03-05T13:04:05", (rows, err))
            check("4  a duration parameter is its ISO text", rows is not None and rows[0]["du"] == "P1Y2M3DT4H", (rows, err))
            rows, err = answer(s, "CREATE (:Event {on: $d, at: $dt})", d=Date(2024, 3, 5),
                               dt=datetime.datetime(2024, 3, 5, 9, 30, tzinfo=datetime.timezone(datetime.timedelta(hours=9))))
            check("5  an event is stored with them", rows == [], (rows, err))
            rows, err = answer(s, "MATCH (e:Event) WHERE date(e.on) = date($d) RETURN toString(e.at) AS at", d=Date(2024, 3, 5))
            check("6  a stored date parameter is found by a date comparison",
                  rows is not None and len(rows) == 1 and "2024-03-05T09:30:00" in rows[0]["at"], (rows, err))
            rows, err = answer(s, "MATCH (e:Event) RETURN e.on AS on")
            check("7  and reads back as the date", rows == [{"on": "2024-03-05"}], (rows, err))
            try:
                s.run("RETURN $p AS p", p=CartesianPoint((1.0, 2.0))).consume()
                refused = None
            except Exception as e:  # noqa: BLE001
                refused = (getattr(e, "code", None), str(e))
            check("8  a point parameter is refused, by name, not stored as a placeholder",
                  refused is not None and refused[0] == "Neo.ClientError.Statement.TypeError" and "point" in refused[1], refused)
            rows, err = answer(s, "RETURN 1 AS x")
            check("9  the session goes on after the refusal", rows == [{"x": 1}], (rows, err))
            answer(s, "CREATE (:Hop {i: 1})-[:NEXT {w: 1}]->(:Hop {i: 2})-[:NEXT {w: 2}]->(:Hop {i: 3})")
            res = s.run("MATCH p = (:Hop {i: 1})-[:NEXT*2]->(:Hop {i: 3}) RETURN p")
            p = res.single()["p"]
            check("10 a path comes back as a Path of three nodes and two relationships",
                  [n["i"] for n in p.nodes] == [1, 2, 3] and [r["w"] for r in p.relationships] == [1, 2], p)
            rows, err = answer(s, "UNWIND $rows AS r MERGE (n:Part {k: r.k}) SET n.at = r.at RETURN count(n) AS c",
                               rows=[{"k": "a", "at": Date(2024, 1, 2)}, {"k": "a", "at": Date(2024, 1, 3)}])
            check("11 dates inside a list of maps reach UNWIND MERGE", rows == [{"c": 2}] , (rows, err))
            rows, err = answer(s, "MATCH (n:Part) RETURN count(n) AS c, collect(n.at) AS at")
            check("12 and the merge found its node the second time", rows == [{"c": 1, "at": ["2024-01-03"]}], (rows, err))
        driver.close()
    finally:
        gw.terminate()
        gw.wait(10)
    print("bolt params: all checks passed")


if __name__ == "__main__":
    main()
