-- Phase 2. ext_a (control without schema -> public, like v1 `ontological`) and ext_b
-- (control schema = 'ext_b_s', like v2 `ontological_age`) in one database. ext_b is built
-- without the fixed-schema module.
\echo === phase 2: two extensions, one database
SHOW search_path;
CREATE EXTENSION ext_a;
CREATE EXTENSION ext_b;
\echo --- extensions and the schema each one was installed into
SELECT e.extname, e.extversion, n.nspname AS extschema
  FROM pg_extension e JOIN pg_namespace n ON n.oid = e.extnamespace
 WHERE e.extname LIKE 'ext_%' ORDER BY 1;
\echo --- every function the two installed: schema, library, C symbol
SELECT e.extname, n.nspname AS schema, p.proname, p.probin, p.prosrc
  FROM pg_proc p
  JOIN pg_namespace n ON n.oid = p.pronamespace
  JOIN pg_depend d ON d.classid = 'pg_proc'::regclass AND d.objid = p.oid AND d.deptype = 'e'
  JOIN pg_extension e ON e.oid = d.refobjid
 WHERE e.extname LIKE 'ext_%'
 ORDER BY p.proname, e.extname;
\echo --- same SQL name, same C symbol name, two shared objects: each call reaches its own
SELECT public.k0_whoami() AS public_k0_whoami, ext_b_s.k0_whoami() AS ext_b_s_k0_whoami;
SELECT public.k0_hello() AS a, ext_b_s.k0_hello() AS b;
SELECT public.k0_edition_local() AS a, ext_b_s.k0_edition_local() AS b;
SELECT public.ext_a_only() AS a, ext_b_s.ext_b_only() AS b;
SELECT k0_fixed.k0_in_fixed_schema() AS ext_a_fixed_schema_fn;
\echo --- og_shared extension_sql! and extension_sql_file! ran once per extension, in its schema
SELECT 'public' AS schema, note FROM public.k0_shared_marker
UNION ALL SELECT 'ext_b_s', note FROM ext_b_s.k0_shared_marker;
SELECT 'public' AS schema, note, obj_description('public.k0_shared_file_marker'::regclass) AS comment
  FROM public.k0_shared_file_marker
UNION ALL SELECT 'ext_b_s', note, obj_description('ext_b_s.k0_shared_file_marker'::regclass)
  FROM ext_b_s.k0_shared_file_marker;
SELECT 'public' AS schema, note FROM public.k0_finalize UNION ALL SELECT 'ext_b_s', note FROM ext_b_s.k0_finalize;
\echo --- SPI inside the library: an unqualified name follows search_path, not the extension
SELECT ext_b_s.k0_spi_unqualified() AS ext_b_calls_unqualified,
       ext_b_s.k0_spi_qualified()   AS ext_b_calls_own_schema;
SET search_path = ext_b_s, public;
SELECT public.k0_spi_unqualified() AS ext_a_calls_unqualified,
       public.k0_spi_qualified()   AS ext_a_calls_own_schema;
RESET search_path;
\echo --- both shared objects are mapped into this one backend
SELECT pg_backend_pid() AS pid \gset
\setenv K0_PID :pid
\! { cat /proc/"$K0_PID"/maps 2>/dev/null || lsof -p "$K0_PID" 2>/dev/null; } | awk '{print $NF}' | grep -E '/ext_[ab]\.(dylib|so)$' | sed 's|.*/||' | sort -u
