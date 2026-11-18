-- Run as supabase_admin; switches to robo with SET ROLE so the checks are robo's.
-- New labels from Cypher CREATE, and what the graph schema's CREATE privilege does.
\pset footer off
SET search_path = ag_catalog, "$user", public;

\echo '### L1 robo graph: Cypher CREATE makes a label table owned by robo'
SET ROLE robo;
SELECT * FROM ag_catalog.cypher('robo_t1', $$ CREATE (p:Policy {name: 'NoNegativeTotal'})-[:GUARDS]->(:Invariant {expr: 'total >= 0'}) RETURN p.name $$) AS (n ag_catalog.agtype);
RESET ROLE;
SELECT l.name, l.kind, c.oid::regclass::text AS tbl, pg_get_userbyid(c.relowner) AS owner,
       (SELECT string_agg(conname || ':' || contype::text, ',') FROM pg_constraint WHERE conrelid = c.oid) AS constraints,
       (SELECT string_agg(indexrelid::regclass::text, ',') FROM pg_index WHERE indrelid = c.oid) AS indexes,
       (SELECT string_agg(tgname, ',') FROM pg_trigger WHERE tgrelid = c.oid AND NOT tgisinternal) AS triggers
  FROM ag_catalog.ag_label l JOIN pg_class c ON c.oid = l.relation
 WHERE l.graph = (SELECT graphid FROM ag_catalog.ag_graph WHERE name = 'robo_t1') AND l.name IN ('Policy', 'GUARDS', '_ag_label_vertex')
 ORDER BY l.id;

\echo '### L2 robo revokes its own CREATE on the graph schema: new labels are refused, existing ones still work'
SET ROLE robo;
REVOKE CREATE ON SCHEMA robo_t1 FROM robo;
SELECT nspacl FROM pg_namespace WHERE nspname = 'robo_t1';
SELECT * FROM ag_catalog.cypher('robo_t1', $$ CREATE (:BrandNew {k: 1}) $$) AS (x ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (a:Aggregate {name: 'Order'}) CREATE (a)-[:BRAND_NEW_REL]->(a) $$) AS (x ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$ CREATE (p:Policy {name: 'AnotherPolicy'}) RETURN p.name $$) AS (x ag_catalog.agtype);
SELECT ag_catalog.create_vlabel('robo_t1', 'BrandNew');
GRANT CREATE ON SCHEMA robo_t1 TO robo;
SELECT * FROM ag_catalog.cypher('robo_t1', $$ CREATE (:BrandNew {k: 1}) RETURN 1 $$) AS (x ag_catalog.agtype);
RESET ROLE;

\echo '### L3 a graph owned by another role (supabase_admin), robo gets USAGE + DML only'
SELECT ag_catalog.create_graph('agesb_shared');
SELECT * FROM ag_catalog.cypher('agesb_shared', $$ CREATE (:Doc {title: 'seed'})-[:LINKS]->(:Doc {title: 'seed2'}) $$) AS (x ag_catalog.agtype);
GRANT USAGE ON SCHEMA agesb_shared TO robo;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA agesb_shared TO robo;
GRANT USAGE ON ALL SEQUENCES IN SCHEMA agesb_shared TO robo;
SET ROLE robo;
SELECT * FROM ag_catalog.cypher('agesb_shared', $$ MATCH (d:Doc)-[:LINKS]->(e) RETURN d.title, e.title $$) AS (a ag_catalog.agtype, b ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('agesb_shared', $$ CREATE (d:Doc {title: 'by robo'}) RETURN d.title $$) AS (a ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('agesb_shared', $$ CREATE (n:NotAllowed {k: 1}) RETURN n $$) AS (a ag_catalog.agtype);
RESET ROLE;
GRANT CREATE ON SCHEMA agesb_shared TO robo;
SET ROLE robo;
SELECT * FROM ag_catalog.cypher('agesb_shared', $$ CREATE (n:NowAllowed {k: 1}) RETURN labels(n) $$) AS (a ag_catalog.agtype);
RESET ROLE;
SELECT c.oid::regclass::text AS tbl, pg_get_userbyid(c.relowner) AS owner
  FROM ag_catalog.ag_label l JOIN pg_class c ON c.oid = l.relation
 WHERE l.graph = (SELECT graphid FROM ag_catalog.ag_graph WHERE name = 'agesb_shared') ORDER BY l.id;

\echo '### L4 table privileges are checked by Cypher DML (AGE 1.8.0)'
REVOKE INSERT ON agesb_shared."Doc" FROM robo;
SET ROLE robo;
SELECT * FROM ag_catalog.cypher('agesb_shared', $$ CREATE (d:Doc {title: 'no insert'}) $$) AS (a ag_catalog.agtype);
RESET ROLE;
REVOKE UPDATE ON agesb_shared."Doc" FROM robo;
SET ROLE robo;
SELECT * FROM ag_catalog.cypher('agesb_shared', $$ MATCH (d:Doc {title: 'by robo'}) SET d.k = 1 $$) AS (a ag_catalog.agtype);
RESET ROLE;
REVOKE SELECT ON agesb_shared."Doc" FROM robo;
SET ROLE robo;
SELECT * FROM ag_catalog.cypher('agesb_shared', $$ MATCH (d:Doc) RETURN count(d) $$) AS (a ag_catalog.agtype);
\echo '-- label-less MATCH reads Doc rows through the parent table _ag_label_vertex'
SELECT * FROM ag_catalog.cypher('agesb_shared', $$ MATCH (n) RETURN labels(n), n.title $$) AS (a ag_catalog.agtype, b ag_catalog.agtype);
RESET ROLE;

\echo '### L5 drop_graph by a non-owner is refused; DROP EXTENSION age is refused by the guard'
SET ROLE robo;
SELECT ag_catalog.drop_graph('agesb_shared', true);
RESET ROLE;
DROP EXTENSION age;
SELECT extname FROM pg_extension WHERE extname = 'age';
SELECT ag_catalog.drop_graph('agesb_shared', true);
