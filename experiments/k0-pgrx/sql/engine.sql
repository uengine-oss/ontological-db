-- The engine's real code under two extension names in one database: `ontological` (engine_v1,
-- control copied from engine/) and `ontological_age` (engine_age, schema oga_catalog). The v2
-- script holds only the 119 pgrx functions (run-engine.sh drops engine/sql/*.sql from it: those
-- create og_catalog / og_data and are v1's own SQL).
\echo === engine: one library, two extensions, one database
SHOW search_path;
CREATE EXTENSION vector;
CREATE EXTENSION ontological;
CREATE EXTENSION ontological_age;
SELECT e.extname, e.extversion, n.nspname AS extschema
  FROM pg_extension e JOIN pg_namespace n ON n.oid = e.extnamespace ORDER BY 1;
\echo --- C functions per extension: schema and library
SELECT e.extname, n.nspname AS schema, count(*) AS c_functions, min(p.probin) AS probin
  FROM pg_proc p
  JOIN pg_namespace n ON n.oid = p.pronamespace
  JOIN pg_depend d ON d.classid = 'pg_proc'::regclass AND d.objid = p.oid AND d.deptype = 'e'
  JOIN pg_extension e ON e.oid = d.refobjid
 WHERE e.extname LIKE 'ontological%' AND p.prolang = (SELECT oid FROM pg_language WHERE lanname = 'c')
 GROUP BY 1, 2 ORDER BY 1, 2;
SELECT n.nspname AS schema, p.proname, p.probin, p.prosrc
  FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
 WHERE p.proname IN ('og_cypher', 'og_create_graph', 'og_cypher_json') ORDER BY 2, 1;
SELECT public.ontological_version() AS v1, oga_catalog.ontological_version() AS v2;
\echo --- a catalog function from each shared object (both write og_catalog: it is v1 code in both)
SELECT public.og_create_graph('k0_v1') AS v1_graph_id, oga_catalog.og_create_graph('k0_v2') AS v2_graph_id;
SELECT graph_id, name FROM og_catalog.graph ORDER BY 1;
\echo --- og_cypher from each shared object
SELECT * FROM public.og_cypher('k0_v1', 'CREATE (:Person {name: ''Ada''}) RETURN 1 AS created');
SELECT * FROM oga_catalog.og_cypher('k0_v1', 'MATCH (p:Person) RETURN p.name AS name');
SELECT public.og_cypher_sql('k0_v1', 'MATCH (p:Person) RETURN p.name AS name')
     = oga_catalog.og_cypher_sql('k0_v1', 'MATCH (p:Person) RETURN p.name AS name') AS same_compiled_sql;
\echo --- v2 og_cypher_json runs og_cypher by SPI, unqualified (engine/src/interop/mod.rs:46): which one?
SET track_functions = 'all';
BEGIN;
SELECT oga_catalog.og_cypher_json('k0_v1', 'MATCH (p:Person) RETURN p.name AS name') AS v2_json;
SELECT schemaname, funcname, calls FROM pg_stat_xact_user_functions
 WHERE funcname IN ('og_cypher', 'og_cypher_json') ORDER BY 2, 1;
COMMIT;
\echo --- both shared objects are mapped into this one backend
SELECT pg_backend_pid() AS pid \gset
\setenv K0_PID :pid
\! { cat /proc/"$K0_PID"/maps 2>/dev/null || lsof -p "$K0_PID" 2>/dev/null; } | awk '{print $NF}' | grep -E '/ontological(_age)?\.(dylib|so)$' | sed 's|.*/||' | sort -u
DROP EXTENSION ontological_age;
DROP EXTENSION ontological CASCADE;   -- the graph data above depends on its schemas
SELECT count(*) AS leftover FROM pg_extension WHERE extname LIKE 'ontological%';
