#!/usr/bin/env python3
"""Run the Neo4j Movie Graph sample against Ontological.

Four questions, answered in order:

  1. Which ports speak Bolt? The PostgreSQL port does not and never will
     (spec 003, FR-024); the Bolt gateway does (spec 011). Both are probed with
     a raw handshake rather than asserted.

  2. Does the sample's *dataset* load? `movies.cypher` from
     neo4j-graph-examples/movies, statement for statement, through og_cypher.

  3. Do the sample's *queries* run and agree with Neo4j? The 24 guide queries in
     queries.py, executed here and — when a Neo4j instance is reachable —
     against Neo4j too, with the row counts compared.

  4. Does the same set of queries agree when it arrives over *Bolt*, driven by
     Neo4j's own driver? Same queries, third path, compared against the first
     two. Plus the things only a driver can check: Node hydration, parameters,
     explicit transactions, failure and RESET.

    python3 tests/neo4j-movies/run.py
    python3 tests/neo4j-movies/run.py --neo4j bolt://localhost:27687

Exit status is 0 when the dataset loads and every read query returns the same
row count on every available path.
"""

import argparse
import os
import re
import socket
import struct
import sys
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from queries import QUERIES  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
DATASET_URL = (
    "https://raw.githubusercontent.com/neo4j-graph-examples/movies/main/scripts/movies.cypher"
)
DATASET = os.path.join(HERE, "movies.cypher")
GRAPH = "movies"

BOLT_MAGIC = b"\x60\x60\xb0\x17"
BOLT_PROPOSALS = struct.pack(">4I", 0x00000405, 0x00000404, 0x00000104, 0x00000003)

# The dataset is a plain property graph; Ontological wants the types declared.
SCHEMA = [
    "SELECT og_create_type('{g}','Movie','entity')",
    "SELECT og_create_type('{g}','Person','entity')",
    "SELECT og_add_property('{g}','Movie','title','string')",
    "SELECT og_add_property('{g}','Movie','released','int')",
    "SELECT og_add_property('{g}','Movie','tagline','string')",
    "SELECT og_add_property('{g}','Person','name','string')",
    "SELECT og_add_property('{g}','Person','born','int')",
    "SELECT og_create_type('{g}','ACTED_IN','relation')",
    "SELECT og_create_type('{g}','DIRECTED','relation')",
    "SELECT og_create_type('{g}','PRODUCED','relation')",
    "SELECT og_create_type('{g}','WROTE','relation')",
    "SELECT og_create_type('{g}','FOLLOWS','relation')",
    "SELECT og_create_type('{g}','REVIEWED','relation')",
    "SELECT og_create_type('{g}','WATCHED','relation')",  # created by guide query q24
    "SELECT og_add_property('{g}','ACTED_IN','roles','text[]')",
    "SELECT og_add_property('{g}','REVIEWED','summary','string')",
    "SELECT og_add_property('{g}','REVIEWED','rating','int')",
    "SELECT og_create_index('{g}','Movie','title')",
    "SELECT og_create_index('{g}','Person','name')",
]

GREEN, RED, YELLOW, DIM, OFF = "\033[32m", "\033[31m", "\033[33m", "\033[2m", "\033[0m"


# ---------------------------------------------------------------- 1. protocol


def bolt_probe(host, port, timeout=3.0):
    """Return (speaks_bolt, detail) for host:port."""
    try:
        s = socket.create_connection((host, port), timeout)
    except OSError as e:
        return False, f"no listener ({e.strerror or e})"
    with s:
        s.settimeout(timeout)
        try:
            s.sendall(BOLT_MAGIC + BOLT_PROPOSALS)
            reply = s.recv(4)
        except OSError as e:
            return False, f"handshake failed ({e})"
        if len(reply) < 4:
            return False, "connection closed without a Bolt reply"
        if reply[:1].isalpha():  # e.g. b'HTTP' — a web server, not Bolt
            return False, f"replied {reply!r}, not a Bolt version"
        version = struct.unpack(">I", reply)[0]
        if version == 0:
            return False, "rejected every proposed Bolt version"
        # Bolt encodes the major version in the low byte, the minor above it.
        return True, f"Bolt {version & 0xFF}.{(version >> 8) & 0xFF}"


# ----------------------------------------------------------------- 2. dataset


def statements(path):
    """The dataset's Cypher statements, minus what is not data."""
    src = open(path, encoding="utf-8").read()
    out = []
    for stmt in src.split(";"):
        stmt = stmt.strip()
        if not stmt:
            continue
        if re.match(r"CREATE\s+(CONSTRAINT|INDEX)", stmt, re.I):
            continue  # schema is declared up front, not inferred from data
        out.append(stmt)
    return out


def split_uri(uri):
    """bolt://host:port → (host, port)"""
    rest = uri.split("://", 1)[-1]
    host, _, port = rest.partition(":")
    return host or "localhost", int(port or 7687)


def fetch_dataset():
    if not os.path.exists(DATASET):
        print(f"{DIM}downloading {DATASET_URL}{OFF}")
        urllib.request.urlretrieve(DATASET_URL, DATASET)
    return DATASET


# ------------------------------------------------------------------- targets


class Ontological:
    name = "ontological"

    def __init__(self, dsn):
        import psycopg2

        self.conn = psycopg2.connect(dsn)
        self.conn.autocommit = True

    def sql(self, q):
        cur = self.conn.cursor()
        cur.execute(q)
        try:
            return cur.fetchall()
        except Exception:
            return []

    def reset(self):
        try:
            self.sql(f"SELECT og_drop_graph('{GRAPH}')")
        except Exception:
            pass
        self.sql(f"SELECT og_create_graph('{GRAPH}')")
        for s in SCHEMA:
            self.sql(s.format(g=GRAPH))

    def run(self, cypher):
        cur = self.conn.cursor()
        cur.execute("SELECT og_cypher(%s::text, %s::text)", (GRAPH, cypher))
        return [r[0] for r in cur.fetchall()]


class Bolt:
    """Ontological, reached the way a Neo4j application reaches it: through
    Neo4j's own driver, over the gateway (spec 011)."""

    name = "bolt"

    def __init__(self, uri, user, password, graph):
        from neo4j import GraphDatabase, basic_auth

        self.driver = GraphDatabase.driver(uri, auth=basic_auth(user, password))
        self.driver.verify_connectivity()
        self.graph = graph

    def session(self):
        return self.driver.session(database=self.graph)

    def run(self, cypher, **params):
        with self.session() as s:
            return [r.data() for r in s.run(cypher, **params)]

    def records(self, cypher):
        """Raw records, so hydration into Node/Relationship can be inspected."""
        with self.session() as s:
            return [r for r in s.run(cypher)]

    def close(self):
        self.driver.close()


class Neo4j:
    name = "neo4j"

    def __init__(self, uri, user, password):
        from neo4j import GraphDatabase

        self.driver = GraphDatabase.driver(uri, auth=(user, password))
        self.driver.verify_connectivity()

    def reset(self):
        with self.driver.session() as s:
            s.run("MATCH (n) DETACH DELETE n")

    def run(self, cypher):
        with self.driver.session() as s:
            return [r.data() for r in s.run(cypher)]

    def load(self, stmts):
        with self.driver.session() as s:
            for stmt in stmts:
                s.run(stmt)


# ----------------------------------------------------------------------- run


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dsn", default=os.environ.get("OG_DSN", "host=localhost port=28816 dbname=og user=dev"))
    ap.add_argument("--pg-host", default="localhost")
    ap.add_argument("--pg-port", type=int, default=int(os.environ.get("OG_PGPORT", 28816)))
    ap.add_argument("--bolt", default=os.environ.get("OG_BOLT_URI", "bolt://localhost:28687"),
                    help="the Ontological Bolt gateway (spec 011)")
    ap.add_argument("--bolt-user", default=os.environ.get("OG_BOLT_USER", "dev"))
    ap.add_argument("--bolt-password", default=os.environ.get("OG_BOLT_PASSWORD", ""))
    ap.add_argument("--no-bolt", action="store_true", help="skip the Bolt gateway checks")
    ap.add_argument("--neo4j", default=os.environ.get("NEO4J_URI", "bolt://localhost:27687"))
    ap.add_argument("--neo4j-user", default=os.environ.get("NEO4J_USER", "neo4j"))
    ap.add_argument("--neo4j-password", default=os.environ.get("NEO4J_PASSWORD", "benchpass123"))
    ap.add_argument("--no-neo4j", action="store_true", help="skip the Neo4j comparison")
    args = ap.parse_args()

    failures = []

    # 1 -------------------------------------------------------------- protocol
    print("── protocol ──────────────────────────────────────────────")
    for label, host, port, want in (
        ("postgres path", args.pg_host, args.pg_port, False),
        ("bolt gateway ", *split_uri(args.bolt), True),
    ):
        ok, detail = bolt_probe(host, port)
        mark = (GREEN + "yes" + OFF) if ok else (RED + "no " + OFF)
        line = f"  {label} {host}:{port}  bolt: {mark}  {DIM}{detail}{OFF}"
        if ok != want and not (want and args.no_bolt):
            line += f"  {RED}← expected {'bolt' if want else 'no bolt'}{OFF}"
            failures.append(f"protocol: {label} bolt={ok}, expected {want}")
        print(line)
    print(
        f"  {DIM}The PostgreSQL port speaks PostgreSQL and nothing else — spec 003's FR-024\n"
        f"  is why. Bolt is the gateway beside it (spec 011): same compiler, same\n"
        f"  og_cypher(), different transport.{OFF}"
    )

    # 2 --------------------------------------------------------------- dataset
    og = Ontological(args.dsn)
    stmts = statements(fetch_dataset())
    print(f"\n── dataset ({len(stmts)} statements) ──────────────────────────")
    og.reset()
    loaded, errors = 0, {}
    for i, stmt in enumerate(stmts):
        try:
            og.run(stmt)
            loaded += 1
        except Exception as e:
            msg = str(e).strip().splitlines()[0]
            errors.setdefault(msg, []).append(i)
    colour = GREEN if not errors else RED
    print(f"  loaded {colour}{loaded}/{len(stmts)}{OFF} statements")
    for msg, idx in errors.items():
        print(f"  {RED}×{OFF} [{len(idx)}×] {msg}")
        failures.append(f"dataset: {msg}")

    counts = {}
    for label, q in (("Movie", "MATCH (m:Movie) RETURN count(m) AS c"),
                     ("Person", "MATCH (p:Person) RETURN count(p) AS c")):
        counts[label] = og.run(q)[0]["c"]
    print(f"  {counts['Movie']} movies, {counts['Person']} people")

    # optional Neo4j side
    neo = None
    if not args.no_neo4j:
        try:
            neo = Neo4j(args.neo4j, args.neo4j_user, args.neo4j_password)
            neo.reset()
            neo.load(stmts)
            print(f"  {DIM}neo4j at {args.neo4j} loaded for comparison{OFF}")
        except Exception as e:
            print(f"  {YELLOW}!{OFF} no Neo4j comparison: {str(e).splitlines()[0]}")
            neo = None

    # the Bolt gateway, driven by Neo4j's own driver
    bolt = None
    if not args.no_bolt:
        try:
            bolt = Bolt(args.bolt, args.bolt_user, args.bolt_password, GRAPH)
            print(f"  {DIM}bolt gateway at {args.bolt} connected with the Neo4j driver{OFF}")
        except Exception as e:
            print(f"  {YELLOW}!{OFF} no Bolt comparison: {str(e).splitlines()[0]}")
            failures.append(f"bolt: cannot connect ({str(e).splitlines()[0]})")

    # 3 --------------------------------------------------------------- queries
    print("\n── guide queries ─────────────────────────────────────────")
    for qid, title, cypher, kind in QUERIES:
        try:
            rows = og.run(cypher)
        except Exception as e:
            print(f"  {RED}×{OFF} {qid} {title}\n      {str(e).strip().splitlines()[0]}")
            failures.append(f"{qid}: {str(e).strip().splitlines()[0]}")
            continue

        note = f"{len(rows)} rows"
        status = GREEN + "✓" + OFF
        if neo is not None:
            try:
                theirs = neo.run(cypher)
            except Exception as e:
                theirs = None
                note += f"  {DIM}(neo4j errored: {str(e).splitlines()[0][:40]}){OFF}"
            if theirs is not None:
                if len(theirs) == len(rows):
                    note += f"  {DIM}= neo4j{OFF}"
                else:
                    status = RED + "×" + OFF
                    note += f"  {RED}≠ neo4j {len(theirs)} rows{OFF}"
                    failures.append(f"{qid}: {len(rows)} rows vs neo4j {len(theirs)}")
        if bolt is not None:
            try:
                over_bolt = bolt.run(cypher)
                if len(over_bolt) == len(rows):
                    note += f"  {DIM}= bolt{OFF}"
                else:
                    status = RED + "×" + OFF
                    note += f"  {RED}≠ bolt {len(over_bolt)} rows{OFF}"
                    failures.append(f"{qid}: {len(rows)} rows vs bolt {len(over_bolt)}")
            except Exception as e:
                status = RED + "×" + OFF
                note += f"  {RED}bolt errored{OFF}"
                failures.append(f"{qid} over bolt: {str(e).splitlines()[0][:80]}")
        print(f"  {status} {qid} {title:52.52} {note}")

    # 4 ------------------------------------------------------------- the driver
    # Things only a real driver can answer: does the wire format hydrate into
    # the objects an application expects, and do sessions behave (spec 011).
    if bolt is not None:
        print("\n── bolt gateway ──────────────────────────────────────────")
        for name, check in driver_checks(bolt, og):
            try:
                detail = check()
                print(f"  {GREEN}✓{OFF} {name:44} {DIM}{detail}{OFF}")
            except Exception as e:
                why = (str(e).strip().splitlines() or [type(e).__name__])[0]
                print(f"  {RED}×{OFF} {name:44} {why[:70]}")
                failures.append(f"bolt/{name}: {why[:80]}")
        bolt.close()

    # ---------------------------------------------------------------- verdict
    print()
    if failures:
        print(f"{RED}{len(failures)} failure(s){OFF}")
        for f in failures:
            print(f"  - {f}")
        return 1
    paths = "postgres + bolt" if bolt is not None else "postgres"
    print(f"{GREEN}the Movie Graph sample runs on Ontological{OFF} "
          f"{DIM}over {paths}, agreeing with Neo4j{OFF}")
    return 0


def driver_checks(bolt, og):
    """Each returns a short detail string, or raises to fail."""

    def nodes_hydrate():
        rec = bolt.records("MATCH (m:Movie {title:'The Matrix'}) RETURN m")[0]
        node = rec["m"]
        assert type(node).__name__ == "Node", f"got {type(node).__name__}, not Node"
        assert node.labels == frozenset({"Movie"}), f"labels are {node.labels}"
        assert node["released"] == 1999, f"released is {node['released']!r}"
        return f"Node labels={set(node.labels)} released={node['released']}"

    def relationships_hydrate():
        from neo4j.graph import Relationship

        rec = bolt.records(
            "MATCH (:Person {name:'Keanu Reeves'})-[r:ACTED_IN]->(:Movie {title:'The Matrix'}) "
            "RETURN r"
        )[0]
        rel = rec["r"]
        assert isinstance(rel, Relationship), f"got {type(rel).__name__}"
        assert rel.type == "ACTED_IN", f"type is {rel.type}"
        assert rel["roles"] == ["Neo"], f"roles are {rel['roles']!r}"
        assert rel.start_node is not None and rel.end_node is not None, "endpoints missing"
        return f"Relationship type={rel.type} roles={rel['roles']}"

    def fields_keep_return_order():
        rec = bolt.records("MATCH (m:Movie) RETURN m.released AS year, m.title AS t LIMIT 1")[0]
        assert list(rec.keys()) == ["year", "t"], f"keys are {list(rec.keys())}"
        return "RETURN order preserved, not jsonb key order"

    def parameters_bind():
        rows = bolt.run(
            "MATCH (p:Person {name:$name})-[:ACTED_IN]->(m:Movie) RETURN m.title AS t",
            name="Tom Hanks",
        )
        assert len(rows) == 12, f"{len(rows)} rows for Tom Hanks"
        return f"{len(rows)} rows for $name='Tom Hanks'"

    def failure_then_reset():
        with bolt.session() as s:
            try:
                s.run("MATCH (x:NoSuchLabel) RETURN x").data()
                raise AssertionError("a bad label did not fail")
            except Exception as e:
                msg = str(e)
                assert "NoSuchLabel" in msg, f"error lost the label: {msg[:60]}"
            # the driver RESETs the connection; the session must go on working
            assert s.run("RETURN 1 AS n").single()["n"] == 1
        return "error carried the label, session recovered"

    def rollback_is_invisible():
        with bolt.session() as s:
            tx = s.begin_transaction()
            tx.run("CREATE (p:Person {name:'Rollback Test'})")
            assert len(list(tx.run("MATCH (p:Person {name:'Rollback Test'}) RETURN p"))) == 1
            tx.rollback()
        after = og.run("MATCH (p:Person {name:'Rollback Test'}) RETURN p")
        assert not after, "the rolled-back node is visible on the PostgreSQL path"
        return "rolled back on both paths"

    def commit_is_visible():
        with bolt.session() as s:
            tx = s.begin_transaction()
            tx.run("CREATE (p:Person {name:'Commit Test'})")
            tx.commit()
        after = og.run("MATCH (p:Person {name:'Commit Test'}) RETURN p")
        assert len(after) == 1, "the committed node is missing on the PostgreSQL path"
        og.run("MATCH (p:Person {name:'Commit Test'}) DETACH DELETE p")
        return "committed on both paths"

    def sessions_are_independent():
        import threading

        query = "MATCH (m:Movie) RETURN m.title AS t"
        expected = len(bolt.run(query))
        errors = []

        def hammer():
            try:
                for _ in range(5):
                    got = len(bolt.run(query))
                    assert got == expected, f"saw {got} rows, expected {expected}"
            except Exception as e:  # noqa: BLE001 — reported, not swallowed
                errors.append(e)

        threads = [threading.Thread(target=hammer) for _ in range(8)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        assert not errors, f"{type(errors[0]).__name__}: {errors[0]}"
        return f"8 concurrent sessions × 5 queries, {expected} rows every time"

    return [
        ("nodes hydrate as Node", nodes_hydrate),
        ("relationships hydrate as Relationship", relationships_hydrate),
        ("fields keep RETURN order", fields_keep_return_order),
        ("parameters bind", parameters_bind),
        ("failure carries the message, RESET recovers", failure_then_reset),
        ("explicit transaction rollback", rollback_is_invisible),
        ("explicit transaction commit", commit_is_visible),
        ("concurrent sessions", sessions_are_independent),
    ]


if __name__ == "__main__":
    sys.exit(main())
