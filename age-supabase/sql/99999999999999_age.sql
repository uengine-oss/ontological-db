-- Apache AGE on Supabase Postgres: install and base grants.
-- Run as supabase_admin (superuser). Idempotent.
--
-- * Fresh data volume: the image runs this file automatically from
--   /docker-entrypoint-initdb.d/migrations/ (after the stock migrations).
-- * Existing data volume: run it once with scripts/install-age.sh.
--
-- AGE itself is preloaded through shared_preload_libraries (image layer), so no
-- role needs LOAD 'age'. This file does not change any role's or the
-- database's search_path (see README, "search_path").

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
