#!/usr/bin/env python3
"""Statement latency and parallel schema changes, through the Bolt gateway.

An application that declares its classes from a source — a type under its
umbrella label, a virtual node, then the virtual relationships — sends a few
statements per table. Two things made that slow or fragile:

  * every statement cost a fixed ~100 ms in the gateway (the statement watcher
    slept in 100 ms ticks and the statement's end waited for the tick), so
    `RETURN 1` took 100 ms where PostgreSQL answers in under one;
  * doing it from parallel sessions failed — `duplicate key value violates
    unique constraint "type_label_pkey"` when two relabelled the hierarchy at
    once, then deadlocks between two dropping the generated views.

This checks that a statement costs about what the database does, that classes,
virtual nodes and relationships made from parallel sessions all arrive, and
that sessions racing to build the same generated view all get an answer.
(A read compiled just before another session's schema change commits can
still fail once with `relation "og_data.v_N" does not exist` — the change
drops the generated views; that is not covered here.)

    python3 tests/bolt-concurrency/run.py   # needs psql, cargo, the neo4j driver

OG_BOLT_BIN runs a gateway binary that is already built instead of `cargo run`.
Exit status is non-zero on the first failed check.
"""

import os
import socket
import statistics
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor

from neo4j import GraphDatabase

HOST = os.environ.get("PGHOST", "localhost")
PORT = os.environ.get("PGPORT", "28816")
USER = os.environ.get("PGUSER")
DB = "og_bolt_concurrency"
LISTEN = os.environ.get("OG_BOLT_TEST_LISTEN", "127.0.0.1:27691")
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
WORKERS = 8
# table -> (parent table its fk column points at, or None). Twelve generic tables.
TABLES = {
    "region": None, "site": "region", "team": "site", "person": "team", "tool": "site",
    "job": "person", "step": "job", "part": None, "use": "part", "visit": "site",
    "note": "visit", "tag": None,
}


def psql(sql, db=DB):
    cmd = ["psql", "-h", HOST, "-p", PORT, "-d", db, "-tAqX", "-v", "ON_ERROR_STOP=1", "-c", sql]
    if USER:
        cmd[1:1] = ["-U", USER]
    out = subprocess.run(cmd, capture_output=True, text=True)
    if out.returncode != 0:
        sys.exit(f"psql failed: {sql}\n{out.stderr}")
    return out.stdout.strip()


def check(what, ok, detail="", note=""):
    if not ok:
        sys.exit(f"FAIL {what} {detail}")
    print(f"ok   {what}{f'  ({note})' if note else ''}")


def setup():
    psql(f"DROP DATABASE IF EXISTS {DB}", db="postgres")
    psql(f"CREATE DATABASE {DB}", db="postgres")
    psql("CREATE EXTENSION ontological CASCADE")
    psql("CREATE SCHEMA src")
    for t, parent in TABLES.items():
        fk = f", {parent}_id int" if parent else ""
        psql(f"CREATE TABLE src.{t} ({t}_id int PRIMARY KEY, {t}_name text{fk})")
        vals = ", ".join(f"({i}, '{t}{i}'{f', {1 + i % 3}' if parent else ''})" for i in range(1, 7))
        psql(f"INSERT INTO src.{t} VALUES {vals}")
    psql("SELECT og_create_datasource('ext', 'postgres', jsonb_build_object("
         "'host', 'localhost', 'port', current_setting('port'), 'database', current_database(), "
         "'schema', 'src', 'user', current_user, 'password', ''))")
    psql("SELECT og_create_graph('conc')")
    psql("SELECT og_cypher('conc', 'CREATE (:_Entity:Seed {x: 1})')")
    return psql("SELECT current_user")


def gateway():
    env = dict(os.environ, OG_BOLT_LISTEN=LISTEN, OG_BOLT_PGHOST=HOST, OG_BOLT_PGPORT=PORT,
               OG_BOLT_PGDATABASE=DB, OG_BOLT_GRAPH="conc")
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


def label(t):
    return t.capitalize()


def main():
    user = setup()
    gw = gateway()
    try:
        driver = GraphDatabase.driver(f"bolt://{LISTEN}", auth=(user, ""), max_connection_pool_size=32)

        # 1. latency: the gateway adds next to nothing to a statement
        with driver.session() as s:
            s.run("RETURN 1 AS x").consume()
            ts = []
            for _ in range(30):
                t0 = time.perf_counter()
                s.run("RETURN 1 AS x").consume()
                ts.append((time.perf_counter() - t0) * 1000)
        med = statistics.median(ts)
        check("1  RETURN 1 costs well under the old 100 ms tick", med < 25, f"median {med:.1f} ms",
              f"median {med:.1f} ms")
        with driver.session() as s:
            t0 = time.perf_counter()
            for _ in range(20):
                s.run("UNWIND [" + ", ".join(map(str, range(1, 51))) + "] AS i RETURN i").consume()
            each = (time.perf_counter() - t0) * 1000 / 20
        check("1  a statement with a result stream is as quick", each < 25, f"{each:.1f} ms", f"{each:.1f} ms each")

        # 2. a class per table from parallel sessions: declare under the umbrella, then map
        def make_class(t):
            with driver.session() as s:
                eid = s.run(f"CREATE (n:_Entity:{label(t)} {{{t}_id: 0}}) RETURN elementId(n) AS e").single()["e"]
                s.run(f"MATCH (n:_Entity:{label(t)}) WHERE elementId(n) = $e DELETE n", e=eid).consume()
                s.run(f"CREATE VIRTUAL NODE {label(t)} IN conc FROM ext.{t} KEY {t}_id").consume()
        errors = []
        with ThreadPoolExecutor(WORKERS) as ex:
            for f in [ex.submit(make_class, t) for t in TABLES]:
                try:
                    f.result()
                except Exception as e:  # noqa: BLE001
                    errors.append(str(e)[:300])
        check("2  classes and virtual nodes from 8 parallel sessions", not errors, errors[:2])
        with driver.session() as s:
            nodes = {r["node"] for r in s.run("SHOW VIRTUAL NODES IN conc")}
        check("2  every virtual node is there", {label(t) for t in TABLES} <= nodes, sorted(nodes))

        # 3. the relationships, also in parallel
        rels = [(t, p) for t, p in TABLES.items() if p]

        def make_rel(tp):
            t, p = tp
            with driver.session() as s:
                s.run(f"CREATE VIRTUAL RELATIONSHIP {t.upper()}_OF IN conc "
                      f"FROM {label(t)}({p}_id) TO {label(p)}({p}_id)").consume()
        errors = []
        with ThreadPoolExecutor(WORKERS) as ex:
            for f in [ex.submit(make_rel, x) for x in rels]:
                try:
                    f.result()
                except Exception as e:  # noqa: BLE001
                    errors.append(str(e)[:300])
        check("3  virtual relationships from 8 parallel sessions", not errors, errors[:2])
        with driver.session() as s:
            have = {r["relationship"] for r in s.run("SHOW VIRTUAL RELATIONSHIPS IN conc")}
            check("3  every virtual relationship is there", {f"{t.upper()}_OF" for t, _ in rels} <= have, sorted(have))
            n = s.run("MATCH (j:Job)-[:JOB_OF]->(p:Person)-[:PERSON_OF]->(t:Team) RETURN count(*) AS n").single()["n"]
            check("3  a two-hop join over them", n == 6, n)
            n = s.run("MATCH (n:_Entity) RETURN count(n) AS n").single()["n"]
            check("3  the umbrella holds every class", n == 1 + 6 * len(TABLES), n)

        # 4. many sessions reading a view nobody has built yet
        with driver.session() as s:
            s.run("CREATE (:_Entity:Fresh {x: 1})").consume()   # a schema change drops the views
        def count_all(_):
            with driver.session() as s:
                return s.run("MATCH (n:_Entity) RETURN count(n) AS n").single()["n"]
        with ThreadPoolExecutor(WORKERS) as ex:
            outs = []
            for f in [ex.submit(count_all, i) for i in range(WORKERS * 2)]:
                try:
                    outs.append(f.result())
                except Exception as e:  # noqa: BLE001
                    outs.append(str(e)[:200])
        check("4  sessions racing to build the same view all answer alike",
              all(o == 2 + 6 * len(TABLES) for o in outs), outs[:3])
        driver.close()
    finally:
        gw.terminate()
        gw.wait(10)
    print("all checks passed")


if __name__ == "__main__":
    main()
