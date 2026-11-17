-- Run as robo after 10: a few Neo4j-isms a Bolt client may send (probe only).
\pset footer off
SET search_path = ag_catalog, "$user", public;
\echo '-- multiple labels on CREATE'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ CREATE (n:A:B {k: 1}) RETURN n $$) AS (n ag_catalog.agtype);
\echo '-- label alternatives in MATCH'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (n:Aggregate|Event) RETURN count(n) $$) AS (n ag_catalog.agtype);
\echo '-- shortestPath'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH p = shortestPath((a:BoundedContext)-[*]->(b:Event)) RETURN length(p) $$) AS (n ag_catalog.agtype);
\echo '-- CALL db.labels()'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ CALL db.labels() YIELD label RETURN label $$) AS (n ag_catalog.agtype);
\echo '-- elementId(n) / id(n)'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (n:Event) RETURN elementId(n) LIMIT 1 $$) AS (n ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (n:Event) RETURN id(n) LIMIT 1 $$) AS (n ag_catalog.agtype);
\echo '-- datetime() / timestamp()'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ RETURN datetime() $$) AS (n ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$ RETURN timestamp() > 0 $$) AS (n ag_catalog.agtype);
\echo '-- RETURN * / RETURN n AS alias'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (n:Event) RETURN * LIMIT 1 $$) AS (n ag_catalog.agtype);
\echo '-- undirected pattern, count(*)'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (a:Aggregate)-[r]-(b) RETURN count(*) $$) AS (n ag_catalog.agtype);
\echo '-- EXPLAIN of a Cypher query'
EXPLAIN (COSTS OFF) SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (a:Aggregate)-[:HAS]->(b) RETURN b.name $$) AS (n ag_catalog.agtype);
