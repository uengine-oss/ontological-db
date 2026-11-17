-- Login role for the Robo Architect test: may use AGE and create its own graphs.
-- Run as supabase_admin with psql:
--   psql -v ON_ERROR_STOP=1 -v robo_verifier='SCRAM-SHA-256$4096:...' -f robo-role.sql
-- robo_verifier is a SCRAM-SHA-256 verifier made on the client, so the
-- plain password never reaches the server or its logs (scripts/create-robo-role.sh).
--
-- Not granted on purpose: BYPASSRLS, superuser, membership in any Supabase role,
-- anything in public / auth / storage. The role does not get a search_path
-- setting either (callers qualify ag_catalog names; see README).

SELECT 'CREATE ROLE robo LOGIN'
 WHERE NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'robo') \gexec

ALTER ROLE robo WITH LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS
  PASSWORD :'robo_verifier';

-- AGE: functions, types and operators live in ag_catalog
GRANT USAGE ON SCHEMA ag_catalog TO robo;
-- graph / label catalogs (lets the gateway list graphs and labels)
GRANT SELECT ON ag_catalog.ag_graph, ag_catalog.ag_label TO robo;
-- create_graph() creates a schema owned by the caller
SELECT format('GRANT CONNECT, CREATE ON DATABASE %I TO robo', current_database()) \gexec
