#!/usr/bin/env python3
"""Cypher over virtual nodes, through the Bolt gateway, as a driver sees it.

The engine's SQL regression (tests/v1/sql/21_cypher_virtual_gaps.sql)
checks the answers; this checks the same six forms arrive through a real Neo4j
driver — records, types and, where a query is wrong, the status code:

  1. arithmetic inside an aggregate, directly and through WITH;
  2. list slices `xs[a..b]`;
  3. head() / last() / tail();
  4. label predicates in WHERE;
  5. a virtual hop whose end has no label;
  6. a boolean compared with 0 / 1 — and a type error is a TypeError,
     not `DatabaseNotFound` (which is kept for a graph that does not exist).

    python3 tests/common/bolt-cypher-gaps/run.py   # needs psql, cargo, the neo4j driver

OG_BOLT_BIN runs a gateway binary that is already built instead of `cargo run`.
Exit status is non-zero on the first failed check.
"""

import os
import socket
import subprocess
import sys
import time

from neo4j import GraphDatabase
from neo4j.exceptions import Neo4jError

HOST = os.environ.get("PGHOST", "localhost")
PORT = os.environ.get("PGPORT", "28816")
USER = os.environ.get("PGUSER")
DB = "og_bolt_gaps"
LISTEN = os.environ.get("OG_BOLT_TEST_LISTEN", "127.0.0.1:27690")
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))


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


def setup():
    psql(f"DROP DATABASE IF EXISTS {DB}", db="postgres")
    psql(f"CREATE DATABASE {DB}", db="postgres")
    psql("CREATE EXTENSION ontological CASCADE")
    psql("CREATE SCHEMA src")
    psql("CREATE TABLE src.member (member_id int PRIMARY KEY, name text, active boolean)")
    psql("CREATE TABLE src.loan (loan_id int PRIMARY KEY, member_id int, days int, fee numeric(6,2), discount float8)")
    psql("INSERT INTO src.member VALUES (1, 'Ana', true), (2, 'Ben', false), (3, 'Cho', NULL)")
    psql("INSERT INTO src.loan VALUES (10, 1, 2, 5.50, 0.5), (11, 1, 3, 2.00, NULL), "
         "(12, 2, 1, 10.00, 1.25), (13, 2, NULL, 4.00, 0), (14, 3, 4, 1.25, 0.25)")
    psql("SELECT og_create_graph('lib')")
    psql("SELECT og_create_datasource('loop', 'postgres', jsonb_build_object("
         "'host', 'localhost', 'port', current_setting('port'), 'database', current_database(), "
         "'user', current_user, 'password', ''))")
    psql("SELECT og_map_virtual('lib', 'Member', 'loop', 'src.member', 'member_id')")
    psql("SELECT og_map_virtual('lib', 'Loan', 'loop', 'src.loan', 'loan_id')")
    psql("SELECT og_create_virtual_rel('lib', 'BORROWED_BY', 'Loan', 'member_id', 'Member', 'member_id')")
    # A stored type beside them, for the same forms over stored nodes.
    psql("SELECT og_cypher('lib', 'CREATE (:Shelf {code: ''S1'', slots: 2, width: 1.5, open: true})')")
    psql("SELECT og_cypher('lib', 'CREATE (:Shelf {code: ''S2'', slots: 3, width: 2, open: false})')")
    return psql("SELECT current_user")


def gateway():
    env = dict(os.environ, OG_BOLT_LISTEN=LISTEN, OG_BOLT_PGHOST=HOST, OG_BOLT_PGPORT=PORT,
               OG_BOLT_PGDATABASE=DB, OG_BOLT_GRAPH="lib")
    cmd = [os.environ["OG_BOLT_BIN"]] if os.environ.get("OG_BOLT_BIN") else \
        ["cargo", "run", "-q", "--manifest-path", os.path.join(ROOT, "bolt", "Cargo.toml")]
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


def one(session, query, **params):
    return session.run(query, **params).single()


def failure(session, query):
    try:
        session.run(query).consume()
    except Neo4jError as e:
        return e.code, e.message
    return None, ""


def main():
    user = setup()
    gw = gateway()
    try:
        driver = GraphDatabase.driver(f"bolt://{LISTEN}", auth=(user, ""))
        with driver.session() as s:
            # 1. arithmetic inside aggregates
            r = one(s, "MATCH (l:Loan) RETURN sum(l.days * l.fee) AS s, avg(l.days - l.discount) AS a")
            check("1  sum(a.x * a.y), avg(a.x - a.y)", float(r["s"]) == 32 and abs(r["a"] - 5 / 3) < 1e-9, dict(r))
            r = one(s, "MATCH (l:Loan) WITH l.days * l.fee AS v RETURN sum(v) AS s, max(v) AS m")
            check("1  sum()/max() of a value carried through WITH", float(r["s"]) == 32 and float(r["m"]) == 11, dict(r))
            r = one(s, "MATCH (l:Loan)-[:BORROWED_BY]->(m:Member) WITH m, l "
                       "RETURN m.name AS name, sum(toFloat(l.days) * l.fee) AS s ORDER BY s DESC LIMIT 1")
            check("1  toFloat() inside sum() over a carried node", r["name"] == "Ana" and float(r["s"]) == 17, dict(r))
            r = one(s, "MATCH (x:Shelf) WITH x RETURN sum(x.slots * x.width) AS s")
            check("1  stored nodes", float(r["s"]) == 9, dict(r))

            # 2. list slices
            r = one(s, "MATCH (m:Member) WITH m ORDER BY m.name "
                       "RETURN collect(m.name)[0..2] AS a, collect(m.name)[..1] AS b, collect(m.name)[-1..] AS c")
            check("2  collect(x)[0..n], [..n], [-1..]", (r["a"], r["b"], r["c"]) == (["Ana", "Ben"], ["Ana"], ["Cho"]), dict(r))
            r = one(s, "RETURN $xs[1..] AS rest", xs=[1, 2, 3])
            check("2  a parameter list", r["rest"] == [2, 3], dict(r))

            # 3. head / last / tail
            r = one(s, "MATCH (m:Member) WITH m ORDER BY m.name WITH collect(m.name) AS ns "
                       "RETURN head(ns) AS h, last(ns) AS l, tail(ns) AS t")
            check("3  head(), last(), tail()", (r["h"], r["l"], r["t"]) == ("Ana", "Cho", ["Ben", "Cho"]), dict(r))

            # 4. label predicates
            r = one(s, "MATCH (n:Member) WHERE n:Member OR n:Loan RETURN count(n) AS n")
            check("4  WHERE n:A OR n:B", r["n"] == 3, dict(r))
            r = one(s, "MATCH (n:Loan) WHERE NOT n:Member RETURN count(n) AS n")
            check("4  WHERE NOT n:A", r["n"] == 5, dict(r))
            r = one(s, "MATCH (n:Shelf) WHERE n:Shelf RETURN count(n) AS n")
            check("4  stored nodes", r["n"] == 2, dict(r))

            # 5. a virtual hop whose end has no label
            r = one(s, "MATCH (a:Loan)-[:BORROWED_BY]->(b) RETURN count(b) AS n")
            check("5  (a:A)-[:REL]->(b)", r["n"] == 5, dict(r))
            r = one(s, "MATCH (b)-[:BORROWED_BY]-(a:Member {name: 'Ana'}) RETURN count(b) AS n")
            check("5  (b)-[:REL]-(a:B), undirected", r["n"] == 2, dict(r))
            r = one(s, "MATCH (m:Member) WITH m MATCH (m)<-[:BORROWED_BY]-(l) RETURN count(l) AS n")
            check("5  a node carried through WITH", r["n"] == 5, dict(r))

            # 6. booleans against 0 / 1, and the status of a type error
            r = one(s, "MATCH (m:Member) WHERE m.active = 1 RETURN collect(m.name) AS c")
            check("6  boolean = 1", r["c"] == ["Ana"], dict(r))
            r = one(s, "MATCH (x:Shelf) WHERE x.open = 0 RETURN collect(x.code) AS c")
            check("6  stored boolean = 0", r["c"] == ["S2"], dict(r))
            code, msg = failure(s, "MATCH (m:Member) WHERE m.active = date('2024-01-01') RETURN m")
            check("6  boolean = date is a TypeError", code == "Neo.ClientError.Statement.TypeError", f"{code}: {msg}")
            code, msg = failure(s, "MATCH (m:Member) RETURN sum(m.name * 2) AS x")
            check("6  a string read as a number is a TypeError", code == "Neo.ClientError.Statement.TypeError", f"{code}: {msg}")
            r = one(s, "RETURN 1 AS one")
            check("   the session goes on after a failure", r["one"] == 1)
        with driver.session(database="nowhere") as s:
            code, msg = failure(s, "RETURN 1")
            check("   a graph that does not exist is still DatabaseNotFound",
                  code == "Neo.ClientError.Database.DatabaseNotFound", f"{code}: {msg}")
        driver.close()
    finally:
        gw.terminate()
        gw.wait(10)
    psql(f"DROP DATABASE IF EXISTS {DB}", db="postgres")
    print("bolt cypher gaps: all checks passed")


if __name__ == "__main__":
    main()
