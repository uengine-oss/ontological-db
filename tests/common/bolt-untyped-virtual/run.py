#!/usr/bin/env python3
"""Relationship patterns without a type over virtual relationships, through the
Bolt gateway, as a driver sees them.

The engine's SQL regression (tests/common/sql/30_cypher_untyped_virtual_relationships.sql)
checks the answers; this checks they arrive through a real Neo4j driver:

  * `(c:Crate)-[r]->(d:Depot)` returns the rows of every typed hop between the
    two labels — the same relationships, element ids and types — where it used
    to return none;
  * two relationship types between the same pair are two Relationships, each
    with its own `type`;
  * direction, the undirected form and `[r:A|B]`;
  * a stored edge and a virtual relationship between the same labels;
  * OPTIONAL MATCH keeps a node with nothing, once;
  * a hop that cannot be joined (no type, an unlabelled end at a virtual node)
    is a client error that says why, and the session goes on.

    python3 tests/common/bolt-untyped-virtual/run.py   # needs psql, cargo, the neo4j driver

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
from neo4j.graph import Relationship

HOST = os.environ.get("PGHOST", "localhost")
PORT = os.environ.get("PGPORT", "28816")
USER = os.environ.get("PGUSER")
DB = "og_bolt_untyped_virtual"
LISTEN = os.environ.get("OG_BOLT_TEST_LISTEN", "127.0.0.1:27699")
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
    psql("CREATE SCHEMA yard_src")
    psql("CREATE TABLE yard_src.depot (depot_id int PRIMARY KEY, name text)")
    psql("CREATE TABLE yard_src.crate (crate_id int PRIMARY KEY, home_depot int, last_depot bigint)")
    psql("INSERT INTO yard_src.depot VALUES (1, 'North'), (2, 'South'), (3, 'East'), (4, 'West')")
    psql("INSERT INTO yard_src.crate VALUES (10, 1, 2), (11, 1, 1), (12, 2, NULL), (13, NULL, 3), (14, 9, 3)")
    psql("SELECT og_create_graph('yard')")
    psql("SELECT og_create_datasource('yardsrc', 'postgres', jsonb_build_object("
         "'host', 'localhost', 'port', current_setting('port'), 'database', current_database(), "
         "'user', current_user, 'password', ''))")
    psql("SELECT og_map_virtual('yard', 'Depot', 'yardsrc', 'yard_src.depot', 'depot_id')")
    psql("SELECT og_map_virtual('yard', 'Crate', 'yardsrc', 'yard_src.crate', 'crate_id')")
    psql("SELECT og_create_virtual_rel('yard', 'STORED_AT', 'Crate', 'home_depot', 'Depot', 'depot_id')")
    psql("SELECT og_create_virtual_rel('yard', 'LAST_SEEN_AT', 'Crate', 'last_depot', 'Depot', 'depot_id')")
    psql("SELECT og_create_type('yard', 'Inspector', 'entity')")
    psql("SELECT og_add_property('yard', 'Inspector', 'name', 'string')")
    psql("SELECT og_add_property('yard', 'Inspector', 'depot_code', 'integer')")
    psql("SELECT og_create_virtual_rel('yard', 'ASSIGNED_TO', 'Inspector', 'depot_code', 'Depot', 'depot_id')")
    psql("SELECT og_cypher('yard', 'CREATE (:Inspector {name: ''Ida'', depot_code: 1}), (:Inspector {name: ''Ole'', depot_code: 3})')")
    psql("SELECT og_cypher('yard', 'MATCH (i:Inspector {name: ''Ida''}), (d:Depot {depot_id: 2}) "
         "CREATE (i)-[:VISITED {times: 2}]->(d)')")
    return psql("SELECT current_user")


def gateway():
    env = dict(os.environ, OG_BOLT_LISTEN=LISTEN, OG_BOLT_PGHOST=HOST, OG_BOLT_PGPORT=PORT,
               OG_BOLT_PGDATABASE=DB, OG_BOLT_GRAPH="yard")
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


def rows(session, query, **params):
    return [r.data() for r in session.run(query, **params)]


def one(session, query, **params):
    return session.run(query, **params).single()


def failure(session, query):
    try:
        session.run(query).consume()
    except Neo4jError as e:
        return e.code, e.message
    return None, ""


def triples(session, rel):
    """(crate, type, depot, element id) of every Crate→Depot relationship the pattern finds."""
    found = []
    for rec in session.run(f"MATCH (c:Crate){rel}(d:Depot) RETURN c.crate_id AS c, d.depot_id AS d, r"):
        r = rec["r"]
        if not isinstance(r, Relationship):
            sys.exit(f"FAIL r is not a Relationship: {r!r}")
        found.append((rec["c"], r.type, rec["d"], r.element_id))
    return sorted(found)


def main():
    user = setup()
    gw = gateway()
    try:
        driver = GraphDatabase.driver(f"bolt://{LISTEN}", auth=(user, ""))
        with driver.session() as s:
            # untyped = the union of the typed hops
            untyped = triples(s, "-[r]->")
            typed = sorted(triples(s, "-[r:STORED_AT]->") + triples(s, "-[r:LAST_SEEN_AT]->"))
            check("(a:A)-[r]->(b:B) returns the rows of every typed hop", untyped == typed and len(untyped) == 7,
                  f"{untyped} vs {typed}")
            check("   the same relationships: element ids and types as typed",
                  [t[3] for t in untyped] == [t[3] for t in typed])
            r = rows(s, "MATCH (c:Crate {crate_id: 11})-[r]->(d:Depot {depot_id: 1}) RETURN type(r) AS t ORDER BY t")
            check("two types between one pair are two relationships", r == [{"t": "LAST_SEEN_AT"}, {"t": "STORED_AT"}], r)
            r = rows(s, "MATCH (c:Crate)-[r]->(d:Depot) RETURN type(r) AS t, count(*) AS n ORDER BY t")
            check("type(r) is each row's own type", r == [{"t": "LAST_SEEN_AT", "n": 4}, {"t": "STORED_AT", "n": 3}], r)
            rec = one(s, "MATCH (c:Crate {crate_id: 12})-[r]->(d:Depot) RETURN r, c, d")
            check("   the Relationship's ends are the matched nodes",
                  rec["r"].start_node.element_id == rec["c"].element_id
                  and rec["r"].end_node.element_id == rec["d"].element_id, rec)

            # direction, undirected, alternatives
            r = one(s, "MATCH (d:Depot)<-[r]-(c:Crate) RETURN count(*) AS n")
            check("incoming, from the other end", r["n"] == 7, dict(r))
            r = one(s, "MATCH (c:Crate)<-[r]-(d:Depot) RETURN count(*) AS n")
            check("against the relationships' direction: none", r["n"] == 0, dict(r))
            r = one(s, "MATCH (c:Crate)-[r]-(d:Depot) RETURN count(*) AS n")
            check("undirected (a:A)-[r]-(b:B)", r["n"] == 7, dict(r))
            r = one(s, "MATCH (c:Crate)--(d:Depot) RETURN count(*) AS n")
            check("anonymous undirected (a:A)--(b:B)", r["n"] == 7, dict(r))
            r = one(s, "MATCH (c:Crate)-[r:STORED_AT|LAST_SEEN_AT]->(d:Depot) RETURN count(*) AS n")
            check("[r:A|B]", r["n"] == 7, dict(r))

            # stored and virtual together
            r = rows(s, "MATCH (i:Inspector)-[r]->(d:Depot) RETURN i.name AS i, type(r) AS t, r.times AS x ORDER BY i, t")
            check("a stored edge beside a virtual relationship",
                  r == [{"i": "Ida", "t": "ASSIGNED_TO", "x": None}, {"i": "Ida", "t": "VISITED", "x": 2},
                        {"i": "Ole", "t": "ASSIGNED_TO", "x": None}], r)

            # OPTIONAL MATCH
            r = rows(s, "MATCH (d:Depot) OPTIONAL MATCH (d)<-[r]-(c:Crate) "
                        "RETURN d.depot_id AS d, count(r) AS n, count(*) AS rows ORDER BY d")
            check("OPTIONAL MATCH: a depot with nothing is one row of NULLs",
                  r == [{"d": 1, "n": 3, "rows": 3}, {"d": 2, "n": 2, "rows": 2}, {"d": 3, "n": 2, "rows": 2},
                        {"d": 4, "n": 0, "rows": 1}], r)

            # what cannot be joined is said
            code, msg = failure(s, "MATCH (c:Crate)-[r]->(x) RETURN count(*) AS n")
            check("an unlabelled end at a virtual node is a client error, not 0 rows",
                  (code or "").startswith("Neo.ClientError.") and "needs both ends labelled" in msg, f"{code}: {msg}")
            code, msg = failure(s, "MATCH (c:Crate)-[*1..2]->(d:Depot) RETURN count(*) AS n")
            check("an untyped variable-length hop at a virtual node is refused",
                  (code or "").startswith("Neo.ClientError.") and "variable length" in msg, f"{code}: {msg}")
            r = one(s, "RETURN 1 AS one")
            check("   the session goes on after a failure", r["one"] == 1)
        driver.close()
    finally:
        gw.terminate()
        gw.wait(10)
    psql(f"DROP DATABASE IF EXISTS {DB}", db="postgres")
    print("bolt untyped virtual relationships: all checks passed")


if __name__ == "__main__":
    main()
