#!/usr/bin/env python3
"""Statement time limit and cancellation over Bolt.

A pattern over virtual nodes is a query on a source system, and a cartesian
product of two of them never finishes. This starts the gateway against a
scratch database whose virtual nodes read a loopback datasource, and checks
that such a query

  * ends at its time limit with Neo.ClientError.Transaction.TransactionTimedOut,
    and the same session runs the next query;
  * honours the driver's own timeout and the gateway's OG_BOLT_STATEMENT_TIMEOUT;
  * stops — here and at the source — when the client goes away, when the
    gateway itself dies, and when the client sends RESET.

    python3 tests/bolt-timeout/run.py      # needs psql, cargo, the neo4j driver

OG_BOLT_BIN runs a gateway binary that is already built instead of `cargo run`.
Exit status is non-zero on the first failed check.
"""

import os
import signal
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
DB = "og_bolt_timeout"
LISTEN = os.environ.get("OG_BOLT_TEST_LISTEN", "127.0.0.1:27689")
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
HERE = os.path.dirname(os.path.abspath(__file__))

ENDLESS = "MATCH (a:Item), (b:Line), (c:Line) RETURN count(*) AS n"
JOINED = "MATCH (l:Line)-[:OF_ITEM]->(i:Item) RETURN count(*) AS n"


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
    """Backends of this database still working on the cartesian product —
    the gateway's own and the loopback source's."""
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


def gateway(env_extra=None):
    env = dict(os.environ, OG_BOLT_LISTEN=LISTEN, OG_BOLT_PGHOST=HOST, OG_BOLT_PGPORT=PORT,
               OG_BOLT_PGDATABASE=DB, OG_BOLT_GRAPH="fab", **(env_extra or {}))
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
    psql("CREATE SCHEMA src")
    psql("CREATE TABLE src.item (item_id int PRIMARY KEY, name text)")
    psql("CREATE TABLE src.line (line_id int PRIMARY KEY, item_id int, qty int)")
    psql("INSERT INTO src.item SELECT g, 'item ' || g FROM generate_series(1, 2000) g")
    psql("INSERT INTO src.line SELECT g, 1 + g % 2000, g % 7 FROM generate_series(1, 5000) g")
    psql("SELECT og_create_graph('fab')")
    psql("SELECT og_create_datasource('loop', 'postgres', jsonb_build_object("
         "'host', 'localhost', 'port', current_setting('port'), 'database', current_database(), "
         "'user', current_user, 'password', ''))")
    psql("SELECT og_map_virtual('fab', 'Item', 'loop', 'src.item', 'item_id')")
    psql("SELECT og_map_virtual('fab', 'Line', 'loop', 'src.line', 'line_id')")
    psql("SELECT og_create_virtual_rel('fab', 'OF_ITEM', 'Line', 'item_id', 'Item', 'item_id')")
    return psql("SELECT current_user")


def timed_out(session, query):
    started = time.time()
    try:
        session.run(query).consume()
    except Neo4jError as e:
        return e.code, time.time() - started, e.message
    return None, time.time() - started, ""


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


def _read_msg(sock):
    data = b""
    while True:
        n = struct.unpack(">H", _recv(sock, 2))[0]
        if n == 0:
            return data
        data += _recv(sock, n)


def _recv(sock, n):
    out = b""
    while len(out) < n:
        chunk = sock.recv(n - len(out))
        if not chunk:
            raise ConnectionError("closed")
        out += chunk
    return out


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
    check("engine reports the default limit", psql("SELECT og_statement_timeout()") == "120000")

    gw = gateway()
    try:
        driver = GraphDatabase.driver(f"bolt://{LISTEN}", auth=(user, ""))
        with driver.session(database="fab") as s:
            code, took, msg = timed_out(s, Query(ENDLESS, timeout=2))
            check("cartesian product ends at the driver's timeout",
                  code == "Neo.ClientError.Transaction.TransactionTimedOut" and took < 10,
                  f"code={code} after {took:.1f}s", f"{took:.1f}s, limit 2s")
            check("the error says what to do", "og.statement_timeout" in msg and "cartesian" in msg, msg)
            check("nothing is left running, here or at the source", wait_until(lambda: running() == 0, 5))
            check("the same session answers the next query", s.run(JOINED).single()["n"] == 5000)

            tx = s.begin_transaction(timeout=2)
            try:
                tx.run(ENDLESS).consume()
                code = None
            except Neo4jError as e:
                code = e.code
            check("an explicit transaction's timeout applies to its statements",
                  code == "Neo.ClientError.Transaction.TransactionTimedOut", f"code={code}")
            try:
                tx.close()
            except Exception:
                pass
            check("and the session goes on after it", s.run(JOINED).single()["n"] == 5000)
        driver.close()

        # The client goes away mid-query: Ctrl-C, a killed process, a driver timeout.
        child = subprocess.Popen([sys.executable, os.path.join(HERE, "client.py"), LISTEN, user, ENDLESS],
                                 stderr=subprocess.DEVNULL)
        check("a long query is running", wait_until(lambda: running() > 0, 10))
        time.sleep(1)
        child.send_signal(signal.SIGKILL)
        child.wait()
        gone = time.time()
        check("a client that disconnects takes its query with it (pg_stat_activity)",
              wait_until(lambda: running() == 0, 5), f"still running after {time.time() - gone:.1f}s",
              f"gone {time.time() - gone:.1f}s after the client")

        # RESET is Bolt's interrupt.
        replies, took = raw_reset(user)
        check("RESET interrupts a running query (FAILURE, IGNORED, SUCCESS)",
              replies == [0x7F, 0x7E, 0x70] and took < 10, f"{replies} after {took:.1f}s",
              f"{took:.1f}s incl. 1s before RESET")
        check("and leaves nothing running", wait_until(lambda: running() == 0, 5))
    finally:
        stop(gw)

    # The gateway's own default, from its environment.
    gw = gateway({"OG_BOLT_STATEMENT_TIMEOUT": "2s"})
    try:
        driver = GraphDatabase.driver(f"bolt://{LISTEN}", auth=(user, ""))
        with driver.session(database="fab") as s:
            code, took, msg = timed_out(s, ENDLESS)
            check("OG_BOLT_STATEMENT_TIMEOUT sets the gateway's limit",
                  code == "Neo.ClientError.Transaction.TransactionTimedOut" and took < 10 and "2 s" in msg,
                  f"code={code} after {took:.1f}s: {msg}", f"{took:.1f}s")
            check("a query inside the limit is unaffected", s.run(JOINED).single()["n"] == 5000)
        driver.close()

        # The gateway dies mid-query: PostgreSQL notices its client is gone.
        child = subprocess.Popen([sys.executable, os.path.join(HERE, "client.py"), LISTEN, user, ENDLESS, "600"],
                                 stderr=subprocess.DEVNULL)
        check("a long query is running", wait_until(lambda: running() > 0, 10))
    finally:
        gw.kill()
        gw.wait()
    dead = time.time()
    check("a gateway that dies takes its queries with it (pg_stat_activity)",
          wait_until(lambda: running() == 0, 10), f"still running after {time.time() - dead:.1f}s",
          f"gone {time.time() - dead:.1f}s after the gateway")
    child.kill()
    child.wait()
    print("bolt timeout: all checks passed")


if __name__ == "__main__":
    main()
