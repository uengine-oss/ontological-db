#!/usr/bin/env python3
"""Bolt context injection — spec 014 FR-010, spec 013 FR-009.

A Neo4j driver names who it is acting for, and the instant it wants to read at,
through `tx_metadata` — the channel drivers already have for application
metadata. This starts the gateway against a scratch database, drives it with the
official driver, and checks what landed in the rows, the audit log and the
answers.

    python3 tests/bolt-context/run.py            # needs psql, cargo, the neo4j driver

Exit status is non-zero on the first failed check.
"""

import os
import socket
import subprocess
import sys
import time

from neo4j import GraphDatabase, Query

HOST = os.environ.get("PGHOST", "localhost")
PORT = os.environ.get("PGPORT", "28816")
DB = "og_bolt_ctx"
LISTEN = "127.0.0.1:27688"
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def psql(sql, db=DB):
    out = subprocess.run(
        ["psql", "-h", HOST, "-p", PORT, "-d", db, "-tAqX", "-v", "ON_ERROR_STOP=1", "-c", sql],
        capture_output=True, text=True,
    )
    if out.returncode != 0:
        sys.exit(f"psql failed: {sql}\n{out.stderr}")
    return out.stdout.strip()


def check(what, got, want):
    if got != want:
        sys.exit(f"FAIL {what}: expected {want!r}, got {got!r}")
    print(f"ok   {what}")


def main():
    psql(f"DROP DATABASE IF EXISTS {DB}", db="postgres")
    psql(f"CREATE DATABASE {DB}", db="postgres")
    psql("CREATE EXTENSION ontological CASCADE")
    for stmt in [
        "SELECT og_create_graph('ctx')",
        "SELECT og_create_type('ctx', 'Person', 'entity')",
        "SELECT og_add_property('ctx', 'Person', 'name', 'string')",
        "SELECT og_add_property('ctx', 'Person', 'salary', 'int')",
        "SELECT og_enable_temporal('ctx', 'Person')",
    ]:
        psql(stmt)
    user = psql("SELECT current_user")

    env = dict(os.environ, OG_BOLT_LISTEN=LISTEN, OG_BOLT_PGHOST=HOST, OG_BOLT_PGPORT=PORT,
               OG_BOLT_PGDATABASE=DB, OG_BOLT_GRAPH="ctx")
    gw = subprocess.Popen(["cargo", "run", "-q", "--manifest-path", os.path.join(ROOT, "bolt", "Cargo.toml")],
                          env=env)
    try:
        host, port = LISTEN.split(":")
        for _ in range(600):
            try:
                socket.create_connection((host, int(port)), 0.5).close()
                break
            except OSError:
                time.sleep(0.5)
        else:
            sys.exit("the gateway did not start")

        driver = GraphDatabase.driver(f"bolt://{LISTEN}", auth=(user, ""))
        with driver.session(database="ctx") as s:
            # Auto-commit RUN with metadata.
            s.run(Query("CREATE (:Person {name: 'ann', salary: 100})",
                        metadata={"actor": "agent-9", "source": "bolt-test", "validFrom": "2020-01-01"})).consume()
            meta = psql("SELECT og_meta((og_cypher('ctx', 'MATCH (p:Person) RETURN id(p) AS i')->>'i')::int8)::text")
            check("actor stamped from tx_metadata", '"actor": "agent-9"' in meta, True)
            check("source stamped from tx_metadata", '"source": "bolt-test"' in meta, True)
            check("valid time from tx_metadata",
                  psql("SELECT lower(__valid)::date FROM og_data.n_1"), "2020-01-01")
            check("audit carries the actor",
                  psql("SELECT actor FROM og_data.og_audit WHERE query LIKE '%ann%' ORDER BY audit_id DESC LIMIT 1"),
                  "agent-9")

            # Explicit transaction with metadata.
            tx = s.begin_transaction(metadata={"actor": "agent-10", "validFrom": "2023-01-01"})
            tx.run("MATCH (p:Person {name: 'ann'}) SET p.salary = 200").consume()
            tx.commit()
            check("explicit transaction stamps its actor",
                  psql("SELECT __actor FROM og_data.n_1"), "agent-10")

            # Settings are transaction-local: the next RUN carries none.
            s.run("MATCH (p:Person) RETURN count(p)").consume()
            check("metadata does not leak into the next transaction",
                  psql("SELECT coalesce(actor, '<none>') FROM og_data.og_audit ORDER BY audit_id DESC LIMIT 1"),
                  "<none>")

            # Reading at an instant.
            q = "MATCH (p:Person {name: 'ann'}) RETURN p.salary AS s"
            check("current value", s.run(q).single()["s"], 200)
            check("value at validAt 2021",
                  s.run(Query(q, metadata={"validAt": "2021-06-01"})).single()["s"], 100)
            tx = s.begin_transaction(metadata={"valid_at": "2021-06-01"})
            check("value at valid_at 2021 in an explicit transaction", tx.run(q).single()["s"], 100)
            tx.commit()
        driver.close()
    finally:
        gw.terminate()
        gw.wait()
    print("bolt context: all checks passed")


if __name__ == "__main__":
    main()
