# Neo4j Movie Graph, run against Ontological

The question this answers: **take an existing Neo4j sample application — does it
work here?** The sample is the Movie Graph, the one behind `:play movies` in
Neo4j Browser, from [neo4j-graph-examples/movies][repo]. Nothing in it is
rewritten for this database; rewriting it would answer a different question.

## What the run checks

1. **Which port speaks Bolt.** The PostgreSQL port does not and never will
   (spec 003, FR-024); the Bolt gateway does (spec 011). Both are probed with a
   raw handshake rather than asserted.
2. **The dataset.** `movies.cypher`, statement for statement, through
   `og_cypher`. One difference from Neo4j: node and relationship types are
   declared first (`og_create_type`), because types are part of identity here
   (spec 002). The guide's two `CREATE CONSTRAINT` statements are dropped for
   the same reason — the schema is declared, not inferred.
3. **The queries.** The 24 Cypher queries from the guide's own documentation
   (`documentation/movies.adoc`), in `queries.py`, verbatim — run over the
   PostgreSQL path, over Bolt, and against a live Neo4j, with the row counts
   compared. A query that runs but returns a different count is a failure.
4. **The driver.** What only a real driver can answer: `Node` and
   `Relationship` hydration, field order, parameter binding, failure and
   `RESET` recovery, explicit transaction commit and rollback (checked from the
   *other* path), and eight concurrent sessions.

The Bolt side is driven by **Neo4j's own driver**. Testing our server with a
client we wrote would not be evidence.

## Running it

```bash
# both Ontological paths
python3 tests/neo4j-movies/run.py

# add the Neo4j comparison
docker run -d --name bench-neo4j -p 27687:7687 \
  -e NEO4J_AUTH=neo4j/benchpass123 neo4j:5
python3 tests/neo4j-movies/run.py --neo4j bolt://localhost:27687

# the official sample application, URI changed and nothing else
python3 tests/neo4j-movies/sample_app.py
```

Requires `psycopg2`; `neo4j` for the Bolt and Neo4j phases. `--no-bolt` and
`--no-neo4j` skip those. `movies.cypher` is downloaded on first run and cached
next to this file.

Exit status is 0 when the dataset loads clean, every query agrees on every
available path, and the driver-level checks pass.

## What porting an application actually costs

With the gateway running: the URI. Without it — if you would rather not run a
second process — the connection layer:

```python
# Bolt gateway (spec 011): the application does not change
GraphDatabase.driver("bolt://og-host:7687", auth=basic_auth(role, pw))

# straight PostgreSQL: one layer changes, the Cypher above it does not
psycopg2.connect("host=og-host port=28816").cursor()
        .execute("SELECT og_cypher(%s, %s)", (graph, cypher))
```

Either way the Cypher, the parameters and the result keys carry over unchanged.
`docs/cypher.md` is the authority on what Cypher does *not* carry over — `WITH`,
`UNION` and `shortestPath` fail identically on both paths.

[repo]: https://github.com/neo4j-graph-examples/movies
