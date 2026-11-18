-- Run as supabase_admin: do the stock Supabase extensions still work next to AGE?
\pset footer off
\echo '### preload / versions'
SELECT version();
SHOW shared_preload_libraries;
SHOW session_preload_libraries;
SELECT extname, extversion FROM pg_extension ORDER BY 1;

\echo '### pg_graphql'
SET ROLE anon;
SELECT graphql.resolve($$ { __typename } $$);
RESET ROLE;
SELECT jsonb_array_length(graphql.resolve($$ { __schema { types { name } } } $$) -> 'data' -> '__schema' -> 'types') AS graphql_types;
\echo '-- ag_catalog / graph schemas must not appear in GraphQL (they are not on any role search_path)'
SELECT count(*) AS age_types_in_graphql
  FROM jsonb_array_elements(graphql.resolve($$ { __schema { types { name } } } $$) -> 'data' -> '__schema' -> 'types') t
 WHERE t ->> 'name' ILIKE ANY (ARRAY['%ag_graph%', '%ag_label%', '%agtype%']);

\echo '### pgvector (create, operator, index)'
CREATE EXTENSION IF NOT EXISTS vector WITH SCHEMA extensions;
CREATE TEMP TABLE agesb_vec (id int, v extensions.vector(3));
INSERT INTO agesb_vec VALUES (1, '[1,0,0]'), (2, '[0,1,0]'), (3, '[0.9,0.1,0]');
CREATE INDEX ON agesb_vec USING hnsw (v extensions.vector_l2_ops);
SELECT id, v OPERATOR(extensions.<->) '[1,0,0]' AS dist FROM agesb_vec ORDER BY 2 LIMIT 2;

\echo '### pg_stat_statements'
SELECT count(*) > 0 AS has_rows FROM extensions.pg_stat_statements;

\echo '### pgcrypto / pgjwt / uuid-ossp'
SELECT extensions.crypt('x', extensions.gen_salt('bf')) LIKE '$2a$%' AS bcrypt_ok,
       extensions.uuid_generate_v4() IS NOT NULL AS uuid_ok,
       extensions.sign('{"sub":"1"}'::json, 'secret') LIKE 'eyJ%' AS jwt_ok;

\echo '### vault / pgsodium'
SELECT vault.create_secret('agesb-check', 'agesb_check_secret') IS NOT NULL AS vault_created;
SELECT decrypted_secret FROM vault.decrypted_secrets WHERE name = 'agesb_check_secret';
DELETE FROM vault.secrets WHERE name = 'agesb_check_secret';

\echo '### pg_cron / pg_net workers'
SELECT backend_type FROM pg_stat_activity WHERE backend_type IN ('pg_cron launcher', 'pg_net 0.14.0 worker') ORDER BY 1;
CREATE EXTENSION IF NOT EXISTS pg_cron;
SELECT cron.schedule('agesb-check', '* * * * *', 'select 1') AS cron_job;
SELECT cron.unschedule('agesb-check');
DROP EXTENSION pg_cron;

\echo '### auth helpers used by RLS'
BEGIN;
-- the bootstrap auth.uid() of this image reads request.jwt.claim.sub (GoTrue's
-- migrations later replace it with the request.jwt.claims version)
SELECT set_config('request.jwt.claim.sub', '00000000-0000-0000-0000-000000000001', true) IS NOT NULL AS claim_set,
       set_config('request.jwt.claim.role', 'authenticated', true) IS NOT NULL AS role_set;
SELECT auth.uid(), auth.role();
COMMIT;

\echo '### cleanup of what this check created'
DROP TABLE agesb_vec;
DROP EXTENSION vector;

-- databases without the age extension: see 02-other-database.sql
