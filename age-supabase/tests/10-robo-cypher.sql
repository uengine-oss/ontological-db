-- Run as robo (no LOAD, no SET search_path): Cypher read/write and output shapes.
--   psql -X -e -v ON_ERROR_STOP=0 -f 10-robo-cypher.sql
\pset footer off
\echo '### reset: drop robo_t1 left by an earlier run'
BEGIN;
SET LOCAL search_path = ag_catalog, "$user", public;
SELECT drop_graph(name, true) FROM ag_catalog.ag_graph WHERE name = 'robo_t1';
COMMIT;

\echo '### session'
SELECT current_user, session_user, current_setting('search_path') AS search_path;

\echo '### A. label DDL without ag_catalog on search_path'
SELECT ag_catalog.create_graph('robo_t1');
\echo '-- same call with ag_catalog on the path (SET LOCAL inside a transaction)'
BEGIN;
SET LOCAL search_path = ag_catalog, "$user", public;
SELECT ag_catalog.create_graph('robo_t1');
COMMIT;
SELECT current_setting('search_path') AS search_path_after_commit;
SELECT nspname, nspowner::regrole AS owner FROM pg_namespace WHERE nspname = 'robo_t1';
SELECT name, kind, relation::regclass::text AS tbl, pg_get_userbyid(c.relowner) AS owner
  FROM ag_catalog.ag_label l JOIN pg_class c ON c.oid = l.relation
 WHERE graph = (SELECT graphid FROM ag_catalog.ag_graph WHERE name = 'robo_t1') ORDER BY id;
\echo '-- explicit label functions without the path'
SELECT ag_catalog.create_vlabel('robo_t1', 'VNoPath');
SELECT ag_catalog.create_elabel('robo_t1', 'E_NO_PATH');
\echo '-- Cypher CREATE of a new vertex label / a new edge label without the path'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ CREATE (n:FreshVertex {k: 1}) RETURN n $$) AS (n ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (n:FreshVertex) CREATE (n)-[r:FRESH_EDGE]->(n) RETURN r $$) AS (r ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MERGE (a:FreshVertex {k: 1})-[r:MERGED_EDGE]->(b:FreshVertex {k: 2}) RETURN r $$) AS (r ag_catalog.agtype);
\echo '-- with the path'
BEGIN;
SET LOCAL search_path = ag_catalog, "$user", public;
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (n:FreshVertex) CREATE (n)-[r:FRESH_EDGE]->(n) RETURN r $$) AS (r ag_catalog.agtype);
COMMIT;
\echo '-- once the edge label exists, writes to it work without the path'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (n:FreshVertex) CREATE (n)-[r:FRESH_EDGE {again: true}]->(n) RETURN count(r) $$) AS (c ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (n:FreshVertex) DETACH DELETE n $$) AS (x ag_catalog.agtype);

\echo '### B. CREATE (labels created on the fly; path set for this transaction only)'
BEGIN;
SET LOCAL search_path = ag_catalog, "$user", public;
SELECT * FROM ag_catalog.cypher('robo_t1', $$
  CREATE (o:BoundedContext {name: 'Sales'}),
         (a:Aggregate {name: 'Order', bc: 'Sales', fields: ['id', 'total'], version: 1}),
         (b:Aggregate {name: 'OrderLine', bc: 'Sales', version: 1}),
         (e:Event {name: 'OrderPlaced', at: '2026-11-18T09:00:00Z'}),
         (o)-[:CONTAINS]->(a), (o)-[:CONTAINS]->(b),
         (a)-[:HAS {card: '1..*'}]->(b),
         (a)-[:EMITS]->(e)
  RETURN a, b
$$) AS (a ag_catalog.agtype, b ag_catalog.agtype);
COMMIT;
SELECT name, kind, relation::regclass::text AS tbl FROM ag_catalog.ag_label
 WHERE graph = (SELECT graphid FROM ag_catalog.ag_graph WHERE name = 'robo_t1') ORDER BY id;

\echo '### C. which Cypher works WITHOUT ag_catalog on search_path (qualified ag_catalog.cypher / ag_catalog.agtype only)'
\echo '-- C1 RETURN literal'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ RETURN 1 $$) AS (v ag_catalog.agtype);
\echo '-- C2 single-label scan'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (v:Event) RETURN v.name $$) AS (v ag_catalog.agtype);
\echo '-- C3 WHERE with = and ORDER BY'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (a:Aggregate) WHERE a.name = 'Order' RETURN a.name ORDER BY a.name $$) AS (v ag_catalog.agtype);
\echo '-- C4 count(n) over all vertices'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (n) RETURN count(n) $$) AS (v ag_catalog.agtype);
\echo '-- C5 property map in the pattern (uses @>)'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (a:Aggregate {name: 'Order'}) RETURN a.name $$) AS (v ag_catalog.agtype);
\echo '-- C6 edge pattern (joins on graphid =)'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (a)-[r:HAS]->(b) RETURN type(r) $$) AS (v ag_catalog.agtype);
\echo '-- C7 UNWIND (age_unnest)'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ UNWIND [1, 2] AS x RETURN x $$) AS (v ag_catalog.agtype);
\echo '-- C8 CREATE into an existing label, SET, DELETE (WHERE form)'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ CREATE (e:Event {name: 'np'}) RETURN e.name $$) AS (v ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (e:Event) WHERE e.name = 'np' SET e.k = 1 RETURN e.k $$) AS (v ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (e:Event) WHERE e.name = 'np' DELETE e $$) AS (v ag_catalog.agtype);
\echo '-- C9 MERGE on a property map'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MERGE (e:Event {name: 'np2'}) RETURN e.name $$) AS (v ag_catalog.agtype);

\echo '### D. the rest runs with the session path the gateway sets at connect'
SET search_path = ag_catalog, "$user", public;
\echo '### MATCH: WHERE / ORDER BY / SKIP / LIMIT / functions'
SELECT * FROM ag_catalog.cypher('robo_t1', $$
  MATCH (c:BoundedContext)-[:CONTAINS]->(x:Aggregate)
  WHERE x.version >= 1 AND x.name STARTS WITH 'Order' AND x.bc IN ['Sales', 'Billing']
  RETURN c.name, x.name, id(x), labels(x), properties(x), keys(x), toUpper(x.name), size(x.name)
  ORDER BY x.name DESC SKIP 0 LIMIT 10
$$) AS (c ag_catalog.agtype, x ag_catalog.agtype, i ag_catalog.agtype, l ag_catalog.agtype,
        p ag_catalog.agtype, k ag_catalog.agtype, u ag_catalog.agtype, s ag_catalog.agtype);

SELECT * FROM ag_catalog.cypher('robo_t1', $$
  MATCH (a:Aggregate)-[r]->(b)
  RETURN type(r), startNode(r).name, endNode(r).name, r.card, coalesce(r.card, 'n/a'),
         CASE WHEN r.card IS NULL THEN 'plain' ELSE 'with card' END
  ORDER BY type(r)
$$) AS (t ag_catalog.agtype, s ag_catalog.agtype, e ag_catalog.agtype, c ag_catalog.agtype,
        c2 ag_catalog.agtype, k ag_catalog.agtype);

SELECT * FROM ag_catalog.cypher('robo_t1', $$
  MATCH (n) OPTIONAL MATCH (n)-[r:EMITS]->(e)
  WITH n, count(e) AS emits
  RETURN labels(n)[0] AS label, count(*) AS n, sum(emits) AS emits, collect(n.name) AS names
  ORDER BY label
$$) AS (label ag_catalog.agtype, n ag_catalog.agtype, emits ag_catalog.agtype, names ag_catalog.agtype);

SELECT * FROM ag_catalog.cypher('robo_t1', $$
  UNWIND [1, 2, 3] AS x WITH x WHERE x % 2 = 1
  RETURN x, x * 1.5, toString(x) + '!', [y IN range(1, x) | y * 10], 'Order' =~ 'Ord.*', exists((:Aggregate)-[:EMITS]->())
$$) AS (x ag_catalog.agtype, f ag_catalog.agtype, s ag_catalog.agtype, l ag_catalog.agtype,
        re ag_catalog.agtype, ex ag_catalog.agtype);

\echo '### MERGE (match, then create) with ON CREATE / ON MATCH SET'
SELECT * FROM ag_catalog.cypher('robo_t1', $$
  MERGE (a:Aggregate {name: 'Order'}) ON MATCH SET a.seen = true ON CREATE SET a.created = true RETURN a
$$) AS (a ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$
  MERGE (a:Aggregate {name: 'Invoice'}) ON MATCH SET a.seen = true ON CREATE SET a.created = true RETURN a
$$) AS (a ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$
  MATCH (c:BoundedContext {name: 'Sales'}), (i:Aggregate {name: 'Invoice'})
  MERGE (c)-[r:CONTAINS]->(i) RETURN r
$$) AS (r ag_catalog.agtype);

\echo '### SET / SET += / REMOVE'
SELECT * FROM ag_catalog.cypher('robo_t1', $$
  MATCH (a:Aggregate {name: 'Order'}) SET a.version = a.version + 1, a.tags = ['core'] RETURN a.version, a.tags
$$) AS (v ag_catalog.agtype, t ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$
  MATCH (a:Aggregate {name: 'Order'}) SET a += {owner: 'team-a', version: 3} REMOVE a.seen RETURN a
$$) AS (a ag_catalog.agtype);

\echo '### paths: variable length, nodes(), relationships(), length()'
SELECT * FROM ag_catalog.cypher('robo_t1', $$
  MATCH p = (c:BoundedContext)-[*1..2]->(x) WHERE x.name = 'OrderLine'
  RETURN length(p), [n IN nodes(p) | n.name], [r IN relationships(p) | type(r)]
  ORDER BY length(p)
$$) AS (len ag_catalog.agtype, ns ag_catalog.agtype, rs ag_catalog.agtype);

\echo '### output shapes: agtype text vs ::jsonb vs ::text'
\echo '-- vertex'
SELECT v AS agtype_text, v::jsonb AS as_jsonb, jsonb_typeof(v::jsonb) AS t
  FROM ag_catalog.cypher('robo_t1', $$ MATCH (v:Event) RETURN v $$) AS (v ag_catalog.agtype);
\echo '-- edge'
SELECT r AS agtype_text, r::jsonb AS as_jsonb
  FROM ag_catalog.cypher('robo_t1', $$ MATCH ()-[r:HAS]->() RETURN r $$) AS (r ag_catalog.agtype);
\echo '-- path'
SELECT p AS agtype_text, p::jsonb AS as_jsonb, jsonb_typeof(p::jsonb) AS t
  FROM ag_catalog.cypher('robo_t1', $$
    MATCH p = (:BoundedContext)-[:CONTAINS]->(:Aggregate {name: 'Order'})-[:HAS]->() RETURN p
  $$) AS (p ag_catalog.agtype);
\echo '-- scalars, list, map, null'
SELECT x AS agtype_text, x::jsonb AS as_jsonb, jsonb_typeof(x::jsonb) AS t
  FROM ag_catalog.cypher('robo_t1', $$
    UNWIND [1, -9223372036854775807, 1.5, 1.0, 'str', true, null, [1, 'a'], {k: 1, n: {m: [true]}}] AS x RETURN x
  $$) AS (x ag_catalog.agtype);
\echo '-- numeric / special floats'
SELECT x AS agtype_text, x::jsonb AS as_jsonb
  FROM ag_catalog.cypher('robo_t1', $$ RETURN 12345678901234567890.123::numeric $$) AS (x ag_catalog.agtype);
SELECT x AS agtype_text
  FROM ag_catalog.cypher('robo_t1', $$ RETURN 'NaN'::float $$) AS (x ag_catalog.agtype);
SELECT x::jsonb AS nan_as_jsonb
  FROM ag_catalog.cypher('robo_t1', $$ RETURN 'NaN'::float $$) AS (x ag_catalog.agtype);
SELECT x::jsonb AS inf_as_jsonb
  FROM ag_catalog.cypher('robo_t1', $$ RETURN 'Infinity'::float $$) AS (x ag_catalog.agtype);
\echo '-- a map that holds a vertex (nested annotation)'
SELECT x AS agtype_text, x::jsonb AS as_jsonb
  FROM ag_catalog.cypher('robo_t1', $$ MATCH (v:Event) RETURN {node: v, names: [v.name]} $$) AS (x ag_catalog.agtype);
\echo '-- one jsonb object per row (the wrapper style)'
SELECT jsonb_build_object('a', (a)::jsonb, 'n', (n)::jsonb) AS row_json
  FROM ag_catalog.cypher('robo_t1', $$ MATCH (a:Aggregate) RETURN a, a.name ORDER BY a.name LIMIT 2 $$)
       AS (a ag_catalog.agtype, n ag_catalog.agtype);
\echo '-- agtype -> text cast and scalar casts'
SELECT n::text AS as_text, (v)::text::int AS via_text_int, (v)::int AS direct_int
  FROM ag_catalog.cypher('robo_t1', $$ MATCH (a:Aggregate {name: 'Order'}) RETURN a.name, a.version $$)
       AS (n ag_catalog.agtype, v ag_catalog.agtype);

\echo '### column definition list must match RETURN'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (a:Aggregate) RETURN a.name, a.bc $$) AS (n ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (a:Aggregate) RETURN a.name $$) AS (n ag_catalog.agtype, x ag_catalog.agtype);
\echo '-- write without RETURN still needs one column'
SELECT ag_catalog.create_vlabel('robo_t1', 'Scratch');
SELECT * FROM ag_catalog.cypher('robo_t1', $$ CREATE (:Scratch {k: 1}) $$) AS (x ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$ CREATE (:Scratch {k: 2}) $$) AS ();

\echo '### DELETE / DETACH DELETE'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (s:Scratch) DELETE s $$) AS (x ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (a:Aggregate {name: 'Invoice'}) DELETE a $$) AS (x ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (a:Aggregate {name: 'Invoice'}) DETACH DELETE a RETURN 1 $$) AS (x ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (n) RETURN count(n) $$) AS (c ag_catalog.agtype);

\echo '### search_path: unqualified names fail without ag_catalog on the path'
RESET search_path;
SELECT * FROM cypher('robo_t1', $$ RETURN 1 $$) AS (v agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$ RETURN 1 $$) AS (v agtype);
\echo '-- the same with SET LOCAL inside a transaction'
BEGIN;
SET LOCAL search_path = ag_catalog, "$user", public;
SELECT * FROM cypher('robo_t1', $$ MATCH (a:Aggregate) RETURN count(a) $$) AS (v agtype);
COMMIT;
SELECT current_setting('search_path') AS after_commit;

\echo '### LOAD by a normal role (not needed with preload)'
LOAD 'age';

\echo '### a transaction that fails rolls the graph back'
SET search_path = ag_catalog, "$user", public;
BEGIN;
SELECT * FROM ag_catalog.cypher('robo_t1', $$ CREATE (:Aggregate {name: 'Rollback'}) $$) AS (x ag_catalog.agtype);
SELECT 1/0;
ROLLBACK;
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (a:Aggregate {name: 'Rollback'}) RETURN count(a) $$) AS (c ag_catalog.agtype);
