-- Phase 4. Install order does not matter; the same schema for both does.
\echo === phase 4: reverse order, then both in one schema
DROP EXTENSION ext_b;
DROP EXTENSION ext_a;
DROP SCHEMA IF EXISTS ext_b_s;
CREATE EXTENSION ext_b;
CREATE EXTENSION ext_a;
SELECT e.extname, n.nspname AS extschema FROM pg_extension e JOIN pg_namespace n ON n.oid = e.extnamespace
 WHERE e.extname LIKE 'ext_%' ORDER BY 1;
SELECT public.k0_whoami() AS a, ext_b_s.k0_whoami() AS b;
DROP EXTENSION ext_a;
\echo --- ext_a put into schema ext_b_s: the two scripts now collide
CREATE EXTENSION ext_a SCHEMA ext_b_s;
DROP EXTENSION IF EXISTS ext_a;
DROP EXTENSION ext_b;
SELECT count(*) AS leftover_extensions FROM pg_extension WHERE extname LIKE 'ext_%';
