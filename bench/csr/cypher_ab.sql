-- End-to-end Cypher, with and without the reachability rewrite.
--
-- Both queries ask the same thing. The only difference is that the second binds
-- a relationship variable, which makes path multiplicity observable and so
-- forces the compiler back onto trail enumeration — the A/B is inside one
-- binary, on one connection, against one dataset.
\set ON_ERROR_STOP on
\pset pager off
SET statement_timeout = '300s';

\echo '=== which plan each query gets ==='
SELECT og_cypher_sql('benchg', $$MATCH (a:P {val:7})-[:K*1..4]->(b:P) RETURN count(DISTINCT b)$$)
       ~ 'og_reach\(' AS rewritten;
SELECT og_cypher_sql('benchg', $$MATCH (a:P {val:7})-[e:K*1..4]->(b:P) RETURN count(DISTINCT b)$$)
       ~ 'og_vlp\(' AS not_rewritten;

\echo '=== same answer ==='
SELECT og_cypher('benchg', $$MATCH (a:P {val:7})-[:K*1..4]->(b:P)  RETURN count(DISTINCT b)$$) AS reach,
       og_cypher('benchg', $$MATCH (a:P {val:7})-[e:K*1..4]->(b:P) RETURN count(DISTINCT b)$$) AS vlp;

\timing on
\echo '=== warm ==='
SELECT og_cypher('benchg', $$MATCH (a:P {val:7})-[:K*1..2]->(b:P) RETURN count(DISTINCT b)$$);
SELECT og_cypher('benchg', $$MATCH (a:P {val:7})-[e:K*1..2]->(b:P) RETURN count(DISTINCT b)$$);

\echo '=== depth 2: reach then vlp ==='
SELECT og_cypher('benchg', $$MATCH (a:P {val:7})-[:K*1..2]->(b:P)  RETURN count(DISTINCT b)$$);
SELECT og_cypher('benchg', $$MATCH (a:P {val:7})-[e:K*1..2]->(b:P) RETURN count(DISTINCT b)$$);
\echo '=== depth 3: reach then vlp ==='
SELECT og_cypher('benchg', $$MATCH (a:P {val:7})-[:K*1..3]->(b:P)  RETURN count(DISTINCT b)$$);
SELECT og_cypher('benchg', $$MATCH (a:P {val:7})-[e:K*1..3]->(b:P) RETURN count(DISTINCT b)$$);
\echo '=== depth 4: reach then vlp ==='
SELECT og_cypher('benchg', $$MATCH (a:P {val:7})-[:K*1..4]->(b:P)  RETURN count(DISTINCT b)$$);
SELECT og_cypher('benchg', $$MATCH (a:P {val:7})-[e:K*1..4]->(b:P) RETURN count(DISTINCT b)$$);
\echo '=== depth 5: reach then vlp ==='
SELECT og_cypher('benchg', $$MATCH (a:P {val:7})-[:K*1..5]->(b:P)  RETURN count(DISTINCT b)$$);
SELECT og_cypher('benchg', $$MATCH (a:P {val:7})-[e:K*1..5]->(b:P) RETURN count(DISTINCT b)$$);
\echo '=== depth 8, rewritten only — the other side does not finish ==='
SELECT og_cypher('benchg', $$MATCH (a:P {val:7})-[:K*1..8]->(b:P)  RETURN count(DISTINCT b)$$);
\echo '=== depth 20, rewritten only ==='
SELECT og_cypher('benchg', $$MATCH (a:P {val:7})-[:K*1..20]->(b:P) RETURN count(DISTINCT b)$$);
