-- Apache AGE on Supabase Postgres: install, base grants, the graph database.
-- Run as supabase_admin (superuser) with psql, connected to the `postgres`
-- database. Idempotent.
--
-- * Fresh data volume: the image runs this file automatically from
--   /docker-entrypoint-initdb.d/migrations/ (after the stock migrations and
--   after Compose's 97-_supabase.sql / 99-*.sql, which sort before it).
-- * Existing data volume: run it once with scripts/install-age.sh.
--
-- AGE itself is preloaded through shared_preload_libraries (image layer), so no
-- role needs LOAD 'age'.
--
-- Three parts:
--   1. `postgres`: the extension, catalog grants, a guard against DROP EXTENSION.
--   2. Every other database, and template1: the extension too. AGE 1.8.0's
--      TRUNCATE hook reads ag_catalog without checking that the extension is
--      there, so with AGE preloaded TRUNCATE fails ("schema "ag_catalog" does
--      not exist") in any database without it (Compose's _supabase, which
--      analytics and the pooler use, and every database made later).
--      With it in template1, databases made later from template1 get it too.
--   3. The graph database (AGE_GRAPH_DB, default `graph`): strings sort by code
--      point there (LC_COLLATE C), as in Neo4j. AGE compares agtype strings with
--      the database's default collation, which in Supabase's `postgres` is
--      en_US.UTF-8 ('orderId' < 'Property 1'). LC_CTYPE is C.UTF-8 when the
--      server has it (Unicode upper/lower and character classes in SQL).
--      Set AGE_GRAPH_DB to an empty value to skip this part.

\set ON_ERROR_STOP on
\set QUIET on
\pset tuples_only on
\pset format unaligned
-- the environment of psql (the image's entrypoint, or `docker exec -e` for install-age.sh)
\set age_graph_db `printf '%s' "${AGE_GRAPH_DB-graph}"`

-- ---------------------------------------------------------------- 1. postgres

CREATE EXTENSION IF NOT EXISTS age;          -- creates schema ag_catalog, owned by supabase_admin

-- Graph catalogs are readable by the owner-side roles; browser roles get nothing.
GRANT USAGE ON SCHEMA ag_catalog TO postgres, service_role;
GRANT SELECT ON ag_catalog.ag_graph, ag_catalog.ag_label TO postgres, service_role;

-- DROP EXTENSION age drops every graph first (AGE's utility hook). Studio's
-- extension switch runs as supabase_admin, so guard it with an event trigger:
-- the drop is rolled back unless the session sets age_guard.allow_drop = on.
CREATE OR REPLACE FUNCTION extensions.age_guard_drop()
RETURNS event_trigger
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $fn$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_event_trigger_dropped_objects()
             WHERE object_type = 'extension' AND object_identity = 'age')
     AND coalesce(current_setting('age_guard.allow_drop', true), 'off') <> 'on' THEN
    RAISE EXCEPTION 'DROP EXTENSION age would drop every graph; refused'
      USING HINT = 'SET age_guard.allow_drop = on in this session if this is intended.';
  END IF;
END
$fn$;
REVOKE ALL ON FUNCTION extensions.age_guard_drop() FROM PUBLIC;

DO $do$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_event_trigger WHERE evtname = 'age_guard_drop') THEN
    CREATE EVENT TRIGGER age_guard_drop ON sql_drop
      EXECUTE FUNCTION extensions.age_guard_drop();
  END IF;
END
$do$;

-- ---------------------------------------------------------------- 3a. make the graph database
-- (before part 2, so that it is not made from a template1 that already has age:
-- it comes from template0 anyway, because its collation differs)

SELECT (:'age_graph_db' <> '') AS age_want_graph_db \gset
\if :age_want_graph_db
SELECT NOT EXISTS (SELECT 1 FROM pg_database WHERE datname = :'age_graph_db') AS age_make_graph_db,
       CASE WHEN EXISTS (SELECT 1 FROM pg_collation WHERE collprovider = 'c' AND collname IN ('C.utf8', 'C.UTF-8'))
            THEN 'C.UTF-8' ELSE 'C' END AS age_graph_ctype \gset
\if :age_make_graph_db
CREATE DATABASE :"age_graph_db" TEMPLATE template0 ENCODING 'UTF8' LC_COLLATE 'C' LC_CTYPE :'age_graph_ctype';
\endif
SELECT datcollate <> 'C' AS age_graph_db_not_c FROM pg_database WHERE datname = :'age_graph_db' \gset
\if :age_graph_db_not_c
\echo 'WARNING: database' :"age_graph_db" 'already exists and does not sort with LC_COLLATE C; strings in its graphs sort by its collation'
\endif
-- Cypher needs ag_catalog on the path (AGE's documented setup).
-- This database holds graphs only, so the setting does not reach Supabase's own sessions.
ALTER DATABASE :"age_graph_db" SET search_path = ag_catalog, "$user", public;
-- a role's own setting wins over the database's: Supabase gives supabase_admin and
-- postgres one, so they get one for this database (only search_path, the rest stays)
ALTER ROLE supabase_admin IN DATABASE :"age_graph_db" SET search_path = ag_catalog, "$user", public;
ALTER ROLE postgres IN DATABASE :"age_graph_db" SET search_path = ag_catalog, "$user", public;
-- connect only for the roles granted below and by sql/robo-role.sql
REVOKE CONNECT, TEMPORARY ON DATABASE :"age_graph_db" FROM PUBLIC;
GRANT CONNECT ON DATABASE :"age_graph_db" TO postgres;
\endif

-- ---------------------------------------------------------------- 2. the other databases and template1

-- One "\connect <db>" + CREATE EXTENSION per database, written to a file and read back
-- (psql cannot \connect from \gexec). template0 does not take connections and is left alone.
\set age_tmp `mktemp "${TMPDIR:-/tmp}/age-dbs.XXXXXX"`
\o :age_tmp
SELECT format(E'\\connect %I\nCREATE EXTENSION IF NOT EXISTS age;', datname)
  FROM pg_database
 WHERE datallowconn AND datname <> current_database()
 ORDER BY datname;
\o
\i :age_tmp
\connect postgres
\set age_rm `rm -f :'age_tmp'`

-- ---------------------------------------------------------------- 3b. the graph database's own setup

\if :age_want_graph_db
\connect :"age_graph_db"
CREATE EXTENSION IF NOT EXISTS age;
GRANT USAGE ON SCHEMA ag_catalog TO postgres;
GRANT SELECT ON ag_catalog.ag_graph, ag_catalog.ag_label TO postgres;

-- the same DROP guard as in postgres (this database has no `extensions` schema)
CREATE SCHEMA IF NOT EXISTS age_admin AUTHORIZATION supabase_admin;
REVOKE ALL ON SCHEMA age_admin FROM PUBLIC;
CREATE OR REPLACE FUNCTION age_admin.age_guard_drop()
RETURNS event_trigger
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $fn$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_event_trigger_dropped_objects()
             WHERE object_type = 'extension' AND object_identity = 'age')
     AND coalesce(current_setting('age_guard.allow_drop', true), 'off') <> 'on' THEN
    RAISE EXCEPTION 'DROP EXTENSION age would drop every graph; refused'
      USING HINT = 'SET age_guard.allow_drop = on in this session if this is intended.';
  END IF;
END
$fn$;
REVOKE ALL ON FUNCTION age_admin.age_guard_drop() FROM PUBLIC;
DO $do$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_event_trigger WHERE evtname = 'age_guard_drop') THEN
    CREATE EVENT TRIGGER age_guard_drop ON sql_drop
      EXECUTE FUNCTION age_admin.age_guard_drop();
  END IF;
END
$do$;
\connect postgres
\endif

\pset tuples_only off
\pset format aligned
\unset QUIET
