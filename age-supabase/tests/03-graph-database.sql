-- Run as supabase_admin. The graph database the image makes (AGE_GRAPH_DB, default graph).
\pset footer off
SELECT datname, datcollate, datctype, has_database_privilege('anon', datname, 'CONNECT') AS anon_connect,
       has_database_privilege('authenticator', datname, 'CONNECT') AS authenticator_connect
  FROM pg_database WHERE datname = 'graph';
\c graph
\echo '### search_path (database setting; supabase_admin has its own one for this database)'
SHOW search_path;
SELECT extversion FROM pg_extension WHERE extname = 'age';
\echo '### strings sort by code point (Neo4j order), not as in postgres (en_US.UTF-8)'
SELECT string_agg(s, ' | ' ORDER BY s) AS sql_order
  FROM unnest(ARRAY['orderId', 'Property 1', 'Zeta', 'apple', '_x', '가', 'Ä', 'Ω', '😀']) s;
SELECT * FROM ag_catalog.create_graph('agesb_sort_probe');
SELECT * FROM ag_catalog.cypher('agesb_sort_probe', $$
  UNWIND ['orderId', 'Property 1', 'Zeta', 'apple', '_x', '가', 'Ä', 'Ω', '😀'] AS s
  WITH s ORDER BY s RETURN collect(s)
$$) AS (cypher_order ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('agesb_sort_probe', $$ RETURN 'orderId' < 'Property 1', 'Zeta' < 'apple' $$)
  AS (a ag_catalog.agtype, b ag_catalog.agtype);
SELECT ag_catalog.drop_graph('agesb_sort_probe', true);
\echo '### the DROP guard'
DROP EXTENSION age;
\c postgres
\echo '### the same in postgres: language order'
SELECT string_agg(s, ' | ' ORDER BY s) AS sql_order_postgres
  FROM unnest(ARRAY['orderId', 'Property 1', 'Zeta', 'apple', '_x', '가', 'Ä', 'Ω', '😀']) s;
