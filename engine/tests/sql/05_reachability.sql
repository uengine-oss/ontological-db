-- Reachability traversal: og_reach, og_reach_sql, the backend-local CSR, and
-- the compiler switch that picks reachability over trail enumeration.
--
-- Every assertion here is an *equality against og_vlp*. The faster paths are
-- only worth having if they return what the slow one returns, so nothing in
-- this file measures anything — it only checks answers.
\set ON_ERROR_STOP on
\pset pager off

SELECT og_create_graph('r');
SELECT og_create_type('r','N','entity');
SELECT og_add_property('r','N','name','string');
SELECT og_create_type('r','E','relation');

-- A diamond with a cycle back to the start:
--   a -> b -> d -> a
--   a -> c -> d
-- so d is reachable two ways at depth 2, and a is reachable again at depth 3.
SELECT og_cypher('r', $$CREATE (:N {name:'a'})$$);
SELECT og_cypher('r', $$CREATE (:N {name:'b'})$$);
SELECT og_cypher('r', $$CREATE (:N {name:'c'})$$);
SELECT og_cypher('r', $$CREATE (:N {name:'d'})$$);
SELECT og_cypher('r', $$MATCH (x:N {name:'a'}), (y:N {name:'b'}) CREATE (x)-[:E]->(y)$$);
SELECT og_cypher('r', $$MATCH (x:N {name:'a'}), (y:N {name:'c'}) CREATE (x)-[:E]->(y)$$);
SELECT og_cypher('r', $$MATCH (x:N {name:'b'}), (y:N {name:'d'}) CREATE (x)-[:E]->(y)$$);
SELECT og_cypher('r', $$MATCH (x:N {name:'c'}), (y:N {name:'d'}) CREATE (x)-[:E]->(y)$$);
SELECT og_cypher('r', $$MATCH (x:N {name:'d'}), (y:N {name:'a'}) CREATE (x)-[:E]->(y)$$);

SELECT id AS a FROM og_data.og_node n
  JOIN og_catalog.type t ON t.type_id = n.type_id AND t.name = 'N'
 WHERE og_node_json(n.id) ->> 'name' = 'a' \gset
SELECT og_type_id('r','E') AS et \gset

\echo '=== the three reachability paths agree with og_vlp, depths 1..5 ==='
-- `og_vlp` counts trails, so DISTINCT is what makes the two comparable; the
-- start node counts as reachable once the cycle returns to it.
SELECT d,
       (SELECT count(DISTINCT node) FROM og_vlp(:a, ARRAY[:et]::int4[],'o',1,d)) AS vlp,
       (SELECT count(*) FROM og_reach_sql(:a, ARRAY[:et]::int4[],'o',1,d))       AS reach_sql,
       (SELECT count(*) FROM og_reach(:a, ARRAY[:et]::int4[],'o',1,d))           AS reach
  FROM generate_series(1,5) d;

\echo '=== depth labels are the first depth each node is reached at ==='
SELECT node = :a AS is_start, depth
  FROM og_reach(:a, ARRAY[:et]::int4[],'o',1,4) ORDER BY depth, is_start;

\echo '=== undirected and inbound directions ==='
SELECT (SELECT count(*) FROM og_reach(:a, ARRAY[:et]::int4[],'i',1,3)) AS inbound,
       (SELECT count(*) FROM og_reach(:a, ARRAY[:et]::int4[],'b',1,3)) AS both;

\echo '=== compiled CSR returns the same set ==='
SELECT nodes, edges FROM og_csr_build(ARRAY[:et]::int4[], 'o');
SELECT count(*) AS disagreements FROM (
    SELECT node, depth FROM og_reach(:a, ARRAY[:et]::int4[],'o',1,4)
  EXCEPT
    SELECT node, depth FROM og_csr_reach(:a, 1, 4)
  UNION ALL
    SELECT node, depth FROM og_csr_reach(:a, 1, 4)
  EXCEPT
    SELECT node, depth FROM og_reach(:a, ARRAY[:et]::int4[],'o',1,4)
) x;

\echo '=== shortest path agrees with BFS depth ==='
SELECT id AS dnode FROM og_data.og_node n
  JOIN og_catalog.type t ON t.type_id = n.type_id AND t.name = 'N'
 WHERE og_node_json(n.id) ->> 'name' = 'd' \gset
SELECT og_csr_hops(:a, :dnode) AS hops,
       (SELECT min(depth) FROM og_csr_reach(:a,1,8) WHERE node = :dnode) AS bfs;

SELECT og_csr_drop();

\echo '=== the compiler picks reachability only when no path is observable ==='
-- These run before the ANALYZE below, so the cost rule has no statistics and
-- falls back to depth alone — which at twelve hops says yes. That is on
-- purpose: it isolates the *semantic* condition, which is what these six
-- assertions are about. The estimate itself is asserted separately, after
-- ANALYZE, by `shallow_keeps_vlp`.
SELECT og_cypher_sql('r', $$MATCH (x:N {name:'a'})-[:E*1..12]->(y:N) RETURN count(DISTINCT y)$$)
       LIKE '%og_reach(%' AS count_distinct_uses_reach;
SELECT og_cypher_sql('r', $$MATCH (x:N {name:'a'})-[:E*1..12]->(y:N) RETURN DISTINCT y.name$$)
       LIKE '%og_reach(%' AS distinct_uses_reach;
-- Multiplicity-sensitive: these must keep enumerating trails at any depth.
SELECT og_cypher_sql('r', $$MATCH (x:N {name:'a'})-[:E*1..12]->(y:N) RETURN count(y)$$)
       LIKE '%og_vlp(%' AS plain_count_keeps_vlp;
SELECT og_cypher_sql('r', $$MATCH (x:N {name:'a'})-[:E*1..12]->(y:N) RETURN y.name$$)
       LIKE '%og_vlp(%' AS plain_return_keeps_vlp;
SELECT og_cypher_sql('r', $$MATCH p = (x:N {name:'a'})-[:E*1..12]->(y:N) RETURN DISTINCT p$$)
       LIKE '%og_vlp(%' AS path_variable_keeps_vlp;
SELECT og_cypher_sql('r', $$MATCH (x:N {name:'a'})-[e:E*1..12]->(y:N) RETURN count(DISTINCT y)$$)
       LIKE '%og_vlp(%' AS rel_variable_keeps_vlp;

\echo '=== and a shallow hop is left alone, because the rewrite would cost more ==='
ANALYZE;
SELECT og_cypher_sql('r', $$MATCH (x:N {name:'a'})-[:E*1..2]->(y:N) RETURN count(DISTINCT y)$$)
       LIKE '%og_vlp(%' AS shallow_keeps_vlp;

\echo '=== and answers the same either way ==='
SELECT og_cypher('r', $$MATCH (x:N {name:'a'})-[:E*1..12]->(y:N) RETURN count(DISTINCT y)$$) AS via_reach;
SELECT og_cypher('r', $$MATCH (x:N {name:'a'})-[e:E*1..12]->(y:N) RETURN count(DISTINCT y)$$) AS via_vlp;
SELECT og_cypher('r', $$MATCH (x:N {name:'a'})-[:E*1..3]->(y:N)  RETURN count(y)$$) AS trail_count;

\echo 'OG_TEST_END'
