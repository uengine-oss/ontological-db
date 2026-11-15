#!/usr/bin/env python3
"""QUERY FABRIC over Bolt — SQL over the datasources' tables, as a fabric statement.

A client that thinks in tables sends `QUERY FABRIC SELECT …` where it would
send any other fabric statement, through the same gateway and driver. This
starts the gateway against a scratch database with a loopback datasource and
checks that the statement

  * answers in one record — the result's columns as a list (order and repeated
    names kept), the rows as lists, typed — and is a read;
  * takes its settings as a block with Bolt parameters;
  * is refused, with its SQLSTATE in the message and the Neo4j code that
    SQLSTATE means, when it would write, read past the datasource tables (a
    catalog, the engine's own tables) or call a pg_* function;
  * ends at the driver's transaction timeout, here and at the source, and
    stops on RESET — and the session goes on as itself after all of it.

    python3 tests/v1/bolt-fabric-query/run.py   # needs psql, cargo, the neo4j driver

The source account logs in with a password (og_fabric_pw, as in the SQL suite's
15 and 23): the statement runs as og_source_reader, which is not a superuser.
OG_BOLT_BIN runs a gateway binary that is already built instead of `cargo run`.
Exit status is non-zero on the first failed check.
"""

import os
import re
import socket
import struct
import subprocess
import sys
import time

from neo4j import GraphDatabase, Query
from neo4j.exceptions import Neo4jError

HOST = os.environ.get("PGHOST", "localhost")
PORT = os.environ.get("PGPORT", "28816")
USER = os.environ.get("PGUSER")
DB = "og_bolt_fabric_query"
LISTEN = os.environ.get("OG_BOLT_TEST_LISTEN", "127.0.0.1:27692")
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

ENDLESS = "QUERY FABRIC SELECT count(*) FROM fq.big a, fq.big b, fq.big c"
FIELDS = ["columns", "types", "rows", "row_count", "truncated", "bytes", "elapsed_ms"]


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


def running():
    return int(psql(
        "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() "
        "AND pid <> pg_backend_pid() AND state = 'active' "
        "AND (query LIKE '%og_cypher%' OR application_name = 'postgres_fdw')"))


def wait_until(cond, seconds):
    end = time.time() + seconds
    while time.time() < end:
        if cond():
            return True
        time.sleep(0.2)
    return cond()


def gateway():
    env = dict(os.environ, OG_BOLT_LISTEN=LISTEN, OG_BOLT_PGHOST=HOST, OG_BOLT_PGPORT=PORT,
               OG_BOLT_PGDATABASE=DB, OG_BOLT_GRAPH="fab")
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


def stop(gw):
    gw.terminate()
    try:
        gw.wait(10)
    except subprocess.TimeoutExpired:
        gw.kill()
        gw.wait()


def setup():
    psql(f"DROP DATABASE IF EXISTS {DB}", db="postgres")
    psql(f"CREATE DATABASE {DB}", db="postgres")
    psql("CREATE EXTENSION ontological CASCADE")
    psql("SELECT og_create_graph('fab')")
    psql("CREATE SCHEMA srcq")
    psql("CREATE TABLE srcq.supplier (supplier_id int PRIMARY KEY, name text, rating numeric(4,2), since date, "
         "active boolean, big bigint, doc jsonb)")
    psql("INSERT INTO srcq.supplier VALUES (1, '대성케이블', 4.50, '2020-01-02', true, 9007199254740993, '{\"a\": [1]}'), "
         "(2, 'Acme', 3.25, '2021-06-30', false, 7, NULL), (3, 'Nord', NULL, NULL, true, NULL, '[]')")
    psql("CREATE TABLE srcq.big AS SELECT g AS id, 'item-' || g AS label FROM generate_series(1, 3000) g")
    psql("DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'og_fabric_pw') THEN "
         "CREATE ROLE og_fabric_pw LOGIN; END IF; END $$")
    psql("ALTER ROLE og_fabric_pw PASSWORD 's3cret-pw:1'")
    psql("GRANT USAGE ON SCHEMA srcq TO og_fabric_pw")
    psql("GRANT SELECT ON ALL TABLES IN SCHEMA srcq TO og_fabric_pw")
    psql("GRANT UPDATE ON srcq.supplier TO og_fabric_pw")  # so FOR SHARE is refused here, not at the source
    psql("SELECT og_fabric(format('CREATE DATASOURCE fq WITH ENGINE = ''postgres'', PARAMETERS = %s', "
         "jsonb_build_object('host', '127.0.0.1', 'port', current_setting('port'), 'database', current_database(), "
         "'schema', 'srcq', 'user', 'og_fabric_pw', 'password', 's3cret-pw:1')))")
    return psql("SELECT current_user")


def failure(session, query, params=None, timeout=None):
    """(Neo4j code, SQLSTATE from the message, position, message, seconds) — or Nones if it ran."""
    started = time.time()
    try:
        session.run(Query(query, timeout=timeout) if timeout else query, params or {}).consume()
    except Neo4jError as e:
        m = re.search(r"\[SQLSTATE ([0-9A-Z]{5})(?:, position (\d+))?\]", e.message or "")
        return e.code, m and m.group(1), m and m.group(2) and int(m.group(2)), e.message, time.time() - started
    return None, None, None, "", time.time() - started


# ---------------------------------------------------------------- raw Bolt
# Enough PackStream to send what the driver will not: a RESET while a query runs.

def _ps(v):
    if v is None:
        return b"\xC0"
    if isinstance(v, bool):
        return b"\xC3" if v else b"\xC2"
    if isinstance(v, int):
        return b"\xCB" + struct.pack(">q", v)
    if isinstance(v, str):
        b = v.encode()
        return (bytes([0x80 | len(b)]) if len(b) < 16 else b"\xD1" + struct.pack(">H", len(b))) + b
    if isinstance(v, dict):
        out = bytes([0xA0 | len(v)])
        for k, x in v.items():
            out += _ps(k) + _ps(x)
        return out
    raise TypeError(v)


def _msg(sig, *fields):
    body = bytes([0xB0 | len(fields), sig]) + b"".join(_ps(f) for f in fields)
    return struct.pack(">H", len(body)) + body + b"\x00\x00"


def _recv(sock, n):
    out = b""
    while len(out) < n:
        chunk = sock.recv(n - len(out))
        if not chunk:
            raise ConnectionError("closed")
        out += chunk
    return out


def _read_msg(sock):
    data = b""
    while True:
        n = struct.unpack(">H", _recv(sock, 2))[0]
        if n == 0:
            return data
        data += _recv(sock, n)


def raw_reset(user):
    host, port = LISTEN.split(":")
    s = socket.create_connection((host, int(port)), 10)
    s.sendall(b"\x60\x60\xB0\x17" + struct.pack(">IIII", 0x0404, 0, 0, 0))
    _recv(s, 4)
    s.sendall(_msg(0x01, {"user_agent": "test", "scheme": "basic", "principal": user, "credentials": ""}))
    _read_msg(s)
    started = time.time()
    s.sendall(_msg(0x10, ENDLESS, {}, {}) + _msg(0x3F, {"n": -1}))
    time.sleep(1.0)
    s.sendall(_msg(0x0F))  # RESET
    replies = [_read_msg(s)[1] for _ in range(3)]  # RUN, PULL, RESET
    elapsed = time.time() - started
    s.sendall(_msg(0x02))
    s.close()
    return replies, elapsed


# ------------------------------------------------------------------- main

def main():
    user = setup()
    gw = gateway()
    try:
        driver = GraphDatabase.driver(f"bolt://{LISTEN}", auth=(user, ""))
        with driver.session(database="fab") as s:
            res = s.run("QUERY FABRIC SELECT supplier_id, name, rating, since, active, big, doc, supplier_id "
                        "FROM fq.supplier ORDER BY 1")
            recs = list(res)
            summary = res.consume()
            check("one record, the statement's fields in order", len(recs) == 1 and list(recs[0].keys()) == FIELDS,
                  f"{len(recs)} records, {recs and list(recs[0].keys())}")
            r = recs[0]
            check("the result's columns as a list: order and repeated names kept",
                  r["columns"] == ["supplier_id", "name", "rating", "since", "active", "big", "doc", "supplier_id"],
                  str(r["columns"]))
            check("values typed: numbers, text, dates as written, booleans, long ints as text, json, null",
                  r["rows"][0] == [1, "대성케이블", 4.5, "2020-01-02", True, "9007199254740993", {"a": [1]}, 1]
                  and r["rows"][2] == [3, "Nord", None, None, True, None, [], 3],
                  str(r["rows"]))
            check("it is a read", summary.query_type == "r", summary.query_type)

            r = s.run("QUERY FABRIC WITH {datasource: $ds, max_rows: $n} SELECT id FROM big ORDER BY id",
                      ds="fq", n=10).single()
            check("settings as a block with Bolt parameters; the row cap says it cut",
                  len(r["rows"]) == 10 and r["truncated"] is True and r["row_count"] == 10, str(dict(r))[:200])

            plan = s.run("EXPLAIN QUERY FABRIC SELECT count(*) FROM fq.big").consume()
            check("EXPLAIN asks without running", plan.query_type == "r")

            code, state, pos, msg, _ = failure(s, "QUERY FABRIC DELETE FROM fq.supplier")
            check("a write is refused before it runs (42601)", state == "42601", f"{code} {msg}")
            code, state, pos, msg, _ = failure(s, "QUERY FABRIC SELECT * FROM fq.supplier FOR SHARE")
            check("what the check lets through meets the read-only transaction (25006)", state == "25006", f"{code} {msg}")
            for q in ["SELECT relname FROM pg_class", "SELECT * FROM og_catalog.datasource",
                      "SELECT pg_read_file('postgresql.conf')", "SELECT pg_sleep(0)"]:
                code, state, pos, msg, _ = failure(s, "QUERY FABRIC " + q)
                check(f"refused as Forbidden (42501): {q}",
                      state == "42501" and code == "Neo.ClientError.Security.Forbidden", f"{code} {msg}")
            code, state, pos, msg, _ = failure(s, "QUERY FABRIC WITH {datasource: 'fq'} SELECT nope FROM supplier")
            check("a source error keeps its SQLSTATE and its position in the SQL",
                  state == "42703" and pos == 8 and 'column "nope" does not exist' in msg, f"{code} {msg}")
            code, state, pos, msg, _ = failure(s, "QUERY FABRIC WITH {max_bytes: 1048576} SELECT repeat('x', 2097152)")
            check("the byte cap (54000)", state == "54000", f"{code} {msg}")

            code, state, pos, msg, took = failure(s, ENDLESS, timeout=1)
            check("the driver's timeout ends it (TransactionTimedOut, 57014)",
                  code == "Neo.ClientError.Transaction.TransactionTimedOut" and state == "57014" and took < 8,
                  f"{code} {state} after {took:.1f}s", f"{took:.1f}s, limit 1s")
            check("nothing is left running, here or at the source", wait_until(lambda: running() == 0, 5))

            r = s.run("QUERY FABRIC SELECT current_user AS u").single()
            check("it ran as og_source_reader", r["rows"] == [["og_source_reader"]], str(r["rows"]))
            ds = [x["name"] for x in s.run("SHOW DATASOURCES")]
            n = s.run("RETURN 1 AS n").single()["n"]
            check("and the session goes on as itself: fabric statements, Cypher", ds == ["fq"] and n == 1, f"{ds} {n}")
        driver.close()

        replies, took = raw_reset(user)
        check("RESET interrupts a running QUERY FABRIC (FAILURE, IGNORED, SUCCESS)",
              replies == [0x7F, 0x7E, 0x70] and took < 10, f"{replies} after {took:.1f}s",
              f"{took:.1f}s incl. 1s before RESET")
        check("and leaves nothing running", wait_until(lambda: running() == 0, 5))

        audited = int(psql("SELECT count(*) FROM og_data.og_audit WHERE query LIKE '%QUERY FABRIC SELECT supplier_id, name%'"))
        check("the audit trail has the SQL", audited >= 1, str(audited))
    finally:
        stop(gw)
    psql(f"DROP DATABASE IF EXISTS {DB}", db="postgres")
    print("all checks passed")


if __name__ == "__main__":
    main()
