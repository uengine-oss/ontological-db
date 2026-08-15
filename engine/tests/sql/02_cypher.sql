-- Spec 003 regression: Cypher parse → compile → execute.
\set ON_ERROR_STOP on
\pset pager off

SELECT og_create_graph('social');
SELECT og_create_type('social','Agent','entity', ARRAY[]::text[], true);   -- abstract
SELECT og_create_type('social','Person','entity', ARRAY['Agent']);
SELECT og_create_type('social','Company','entity', ARRAY['Agent']);
SELECT og_add_property('social','Agent','name','string');
SELECT og_add_property('social','Person','age','int');
SELECT og_add_property('social','Company','sector','string');
SELECT og_create_type('social','KNOWS','relation');
SELECT og_add_property('social','KNOWS','since','int');
SELECT og_create_type('social','WORKS_AT','relation');

\echo '=== CREATE via Cypher ==='
SELECT og_cypher('social', $$CREATE (a:Person {name:'Aria', age:34}) RETURN a$$);
SELECT og_cypher('social', $$CREATE (b:Person {name:'Boaz', age:29}) RETURN b.name$$);
SELECT og_cypher('social', $$CREATE (c:Person {name:'Cleo', age:41}) RETURN c.name$$);
SELECT og_cypher('social', $$CREATE (x:Company {name:'Uengine', sector:'software'}) RETURN x.name$$);

\echo '=== MATCH + WHERE + parameters ==='
SELECT og_cypher('social', $$MATCH (p:Person) WHERE p.age > $min RETURN p.name AS name, p.age AS age ORDER BY p.age DESC$$, '{"min":30}');

\echo '=== relationships ==='
SELECT og_cypher('social', $$MATCH (a:Person {name:'Aria'}), (b:Person {name:'Boaz'}) CREATE (a)-[:KNOWS {since:2019}]->(b) RETURN a.name$$);
SELECT og_cypher('social', $$MATCH (a:Person {name:'Boaz'}), (b:Person {name:'Cleo'}) CREATE (a)-[:KNOWS {since:2021}]->(b) RETURN a.name$$);
SELECT og_cypher('social', $$MATCH (a:Person {name:'Aria'}), (c:Company) CREATE (a)-[:WORKS_AT]->(c) RETURN c.name$$);

\echo '=== 1-hop traversal ==='
SELECT og_cypher('social', $$MATCH (a:Person)-[r:KNOWS]->(b:Person) RETURN a.name AS from, b.name AS to, r.since AS since ORDER BY a.name$$);

\echo '=== variable length (trail semantics) ==='
SELECT og_cypher('social', $$MATCH (a:Person {name:'Aria'})-[:KNOWS*1..3]->(x) RETURN x.name AS reached ORDER BY x.name$$);

\echo '=== inheritance: MATCH (:Agent) picks up Person AND Company ==='
SELECT og_cypher('social', $$MATCH (a:Agent) RETURN a.name AS name ORDER BY a.name$$);

\echo '=== aggregate ==='
SELECT og_cypher('social', $$MATCH (p:Person) RETURN count(p) AS people, avg(p.age) AS mean_age$$);

\echo '=== whole node projection ==='
SELECT og_cypher('social', $$MATCH (p:Person) WHERE p.name = 'Aria' RETURN p$$);

\echo '=== SET / DELETE ==='
SELECT og_cypher('social', $$MATCH (p:Person {name:'Cleo'}) SET p.age = 42 RETURN p.age AS age$$);
SELECT og_cypher('social', $$MATCH (p:Person) WHERE p.name = 'Cleo' RETURN p.age AS age$$);

\echo '=== compiled SQL is plain SQL ==='
SELECT og_cypher_sql('social', $$MATCH (a:Person)-[:KNOWS]->(b:Person) WHERE a.age > 30 RETURN b.name$$);

\echo '=== error quality: unknown label ==='
-- EXPECT_ERROR: unknown label, with a spelling suggestion
SELECT og_cypher('social', $$MATCH (p:Persn) RETURN p$$);
