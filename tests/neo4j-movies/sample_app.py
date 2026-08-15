#!/usr/bin/env python3
"""The official Movie Graph sample application, running against Ontological.

Copied from neo4j-graph-examples/movies, `code/python/example.py`. The whole
diff is the two arguments on the driver line — the URI and the credentials,
which is what changing servers always costs. Nothing below it is ours:

    driver = GraphDatabase.driver(
-     "bolt://<HOST>:<BOLTPORT>",
-     auth=basic_auth("<USERNAME>", "<PASSWORD>"))
+     "bolt://localhost:28687",
+     auth=basic_auth("dev", ""))

    python3 tests/neo4j-movies/sample_app.py [bolt://host:port] [user] [password]

Run `run.py` first — it loads the dataset this queries.
"""

import sys

from neo4j import GraphDatabase, basic_auth

uri = sys.argv[1] if len(sys.argv) > 1 else "bolt://localhost:28687"
user = sys.argv[2] if len(sys.argv) > 2 else "dev"
password = sys.argv[3] if len(sys.argv) > 3 else ""

driver = GraphDatabase.driver(uri, auth=basic_auth(user, password))

cypher_query = """
MATCH (movie:Movie {title:$favorite})<-[:ACTED_IN]-(actor)-[:ACTED_IN]->(rec:Movie)
 RETURN distinct rec.title as title LIMIT 20
"""

with driver.session(database="movies") as session:
    results = session.read_transaction(
        lambda tx: tx.run(cypher_query, favorite="The Matrix").data()
    )
    for record in results:
        print(record["title"])

driver.close()
