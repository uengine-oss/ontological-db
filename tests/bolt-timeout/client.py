#!/usr/bin/env python3
"""Runs one query and waits for it — the client `run.py` kills mid-query.

    client.py <host:port> <user> <query> [timeout seconds]
"""
import sys

from neo4j import GraphDatabase, Query

listen, user, query = sys.argv[1:4]
timeout = float(sys.argv[4]) if len(sys.argv) > 4 else 60
driver = GraphDatabase.driver(f"bolt://{listen}", auth=(user, ""))
with driver.session(database="fab") as s:
    s.run(Query(query, timeout=timeout)).consume()
