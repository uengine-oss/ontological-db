#!/usr/bin/env python3
"""A MySQL source's relation kind and primary keys, through the Bolt gateway.

`SHOW TABLES` reports `kind` (table / view) and `primary_key`, `SHOW COLUMNS`
and `SHOW COLUMN STATS` a per-column `primary_key` — read from the source's own
information_schema (TABLES.TABLE_TYPE, KEY_COLUMN_USAGE of PRIMARY) through the
datasource's mysql_fdw server, when it is created or refreshed. A virtual node's
key (`key`) stays separate from the source's primary key.

Needs a MySQL (or MariaDB) server this test may change one database of:

    OG_TEST_MYSQL_HOST / _PORT / _USER / _PASSWORD / _DATABASE   the source and an account on it
    OG_TEST_MYSQL_CONTAINER   a docker container running it; the test then creates its tables
                              there as root (the container's MYSQL_ROOT_PASSWORD), otherwise
                              they must exist already (see SEED below)

Without OG_TEST_MYSQL_HOST it prints why and exits 0. The account's password is
read from the environment and never printed.

    python3 tests/fabric-mysql-catalog/run.py   # needs psql, cargo, the neo4j driver

OG_BOLT_BIN runs a gateway binary that is already built instead of `cargo run`.
Exit status is non-zero on the first failed check.
"""

import json
import os
import socket
import subprocess
import sys
import time

from neo4j import GraphDatabase

HOST = os.environ.get("PGHOST", "localhost")
PORT = os.environ.get("PGPORT", "28816")
USER = os.environ.get("PGUSER")
DB = "og_fabric_mysql_catalog"
LISTEN = os.environ.get("OG_BOLT_TEST_LISTEN", "127.0.0.1:27698")
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

SEED = """
DROP VIEW IF EXISTS pricey;
DROP TABLE IF EXISTS tradeitems, orderparts, eventlog;
CREATE TABLE tradeitems (item_code varchar(20) PRIMARY KEY, title varchar(100), price decimal(8,2));
CREATE TABLE orderparts (order_no int, line_no int, item_code varchar(20), qty int, PRIMARY KEY (order_no, line_no));
CREATE TABLE eventlog (note varchar(50), seq int);
CREATE VIEW pricey AS SELECT item_code, price FROM tradeitems WHERE price > 10;
INSERT INTO tradeitems VALUES ('A1', 'lamp', 12.5), ('B2', 'cord', 3.0);
INSERT INTO orderparts VALUES (1, 1, 'A1', 2), (1, 2, 'B2', 1);
INSERT INTO eventlog VALUES ('x', 1);
"""


def psql(sql, db=DB):
    cmd = ["psql", "-h", HOST, "-p", PORT, "-d", db, "-tAqX", "-v", "ON_ERROR_STOP=1", "-c", sql]
    if USER:
        cmd[1:1] = ["-U", USER]
    out = subprocess.run(cmd, capture_output=True, text=True)
    if out.returncode != 0:
        sys.exit(f"psql failed: {sql}\n{out.stderr}")
    return out.stdout.strip()


def mysql_root(sql, database):
    container = os.environ["OG_TEST_MYSQL_CONTAINER"]
    out = subprocess.run(
        ["docker", "exec", "-i", container, "sh", "-c", f'mysql -uroot -p"$MYSQL_ROOT_PASSWORD" {database} 2>/dev/null'],
        input=sql, capture_output=True, text=True)
    if out.returncode != 0:
        sys.exit(f"mysql failed in {container}: {out.stdout[-300:]}")


def check(what, ok, detail=""):
    if not ok:
        sys.exit(f"FAIL {what} {detail}")
    print(f"ok   {what}")


def gateway():
    env = dict(os.environ, OG_BOLT_LISTEN=LISTEN, OG_BOLT_PGHOST=HOST, OG_BOLT_PGPORT=PORT,
               OG_BOLT_PGDATABASE=DB, OG_BOLT_GRAPH="shop")
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


def main():
    if not os.environ.get("OG_TEST_MYSQL_HOST"):
        print("fabric mysql catalog: skipped — set OG_TEST_MYSQL_HOST (and _PORT, _USER, _PASSWORD, _DATABASE)")
        return
    database = os.environ.get("OG_TEST_MYSQL_DATABASE", "shopsrc")
    if os.environ.get("OG_TEST_MYSQL_CONTAINER"):
        mysql_root(SEED, database)
    params = {
        "host": os.environ["OG_TEST_MYSQL_HOST"],
        "port": os.environ.get("OG_TEST_MYSQL_PORT", "3306"),
        "database": database,
        "user": os.environ["OG_TEST_MYSQL_USER"],
        "password": os.environ.get("OG_TEST_MYSQL_PASSWORD", ""),
    }
    psql(f"DROP DATABASE IF EXISTS {DB}", db="postgres")
    psql(f"CREATE DATABASE {DB}", db="postgres")
    psql("CREATE EXTENSION ontological CASCADE")
    psql("SELECT og_create_graph('shop')")
    user = psql("SELECT current_user")
    gw = gateway()
    try:
        driver = GraphDatabase.driver(f"bolt://{LISTEN}", auth=(user, ""))
        with driver.session() as s:
            res = s.run("CREATE DATASOURCE shop WITH ENGINE = 'mysql', PARAMETERS = {host: $host, port: $port, "
                        "database: $database, user: $user, password: $password}", **params)
            created = res.single()["result"]
            check("1  a MySQL datasource is created through Bolt",
                  "4 tables" in created and params["password"] not in created if params["password"] else "4 tables" in created,
                  created.split(" — ")[0])
            res = s.run("SHOW TABLES FROM shop")
            tables = {r["table"]: r for r in res}
            check("2  SHOW TABLES has kind and primary_key, after the columns it had",
                  list(res.keys())[:3] == ["table", "columns", "virtual_node"] and {"kind", "primary_key"} <= set(res.keys()),
                  res.keys())
            check("3  a table and a view are told apart",
                  tables["shop.tradeitems"]["kind"] == "table" and tables["shop.pricey"]["kind"] == "view",
                  {t: r["kind"] for t, r in tables.items()})
            check("4  primary keys come from the source, composite ones in key order, none as []",
                  tables["shop.tradeitems"]["primary_key"] == ["item_code"]
                  and tables["shop.orderparts"]["primary_key"] == ["order_no", "line_no"]
                  and tables["shop.eventlog"]["primary_key"] == [] and tables["shop.pricey"]["primary_key"] == [],
                  {t: r["primary_key"] for t, r in tables.items()})
            cols = {r["column"]: r for r in s.run("SHOW COLUMNS FROM shop.orderparts")}
            check("5  SHOW COLUMNS marks the key columns",
                  cols["order_no"]["primary_key"] and cols["line_no"]["primary_key"]
                  and cols["qty"]["primary_key"] is False, {c: r["primary_key"] for c, r in cols.items()})
            stats = {r["column"]: r for r in s.run("SHOW COLUMN STATS FROM shop.tradeitems")}
            check("6  SHOW COLUMN STATS does too", stats["item_code"]["primary_key"] is True
                  and stats["title"]["primary_key"] is False, {c: r["primary_key"] for c, r in stats.items()})
            s.run("CREATE VIRTUAL NODE Note FROM shop.eventlog KEY seq").consume()
            cols = {r["column"]: r for r in s.run("SHOW COLUMNS FROM shop.eventlog")}
            check("7  a virtual node's key is not the source's primary key",
                  cols["seq"]["key"] is True and cols["seq"]["primary_key"] is False, {c: (r["key"], r["primary_key"]) for c, r in cols.items()})
            if os.environ.get("OG_TEST_MYSQL_CONTAINER"):
                mysql_root("ALTER TABLE orderparts DROP PRIMARY KEY, ADD PRIMARY KEY (line_no);", database)
                s.run("REFRESH DATASOURCE shop").consume()
                tables = {r["table"]: r for r in s.run("SHOW TABLES FROM shop")}
                check("8  REFRESH DATASOURCE reads the source's catalog again",
                      tables["shop.orderparts"]["primary_key"] == ["line_no"], tables["shop.orderparts"])
                mysql_root("ALTER TABLE orderparts DROP PRIMARY KEY, ADD PRIMARY KEY (order_no, line_no);", database)
        driver.close()
    finally:
        gw.terminate()
        gw.wait(10)
    print("fabric mysql catalog: all checks passed")


if __name__ == "__main__":
    main()
