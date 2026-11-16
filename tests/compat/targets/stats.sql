-- Run by run.py after the fixture is loaded, for every target (both are PostgreSQL).
-- Planner statistics are taken once here and then held: autovacuum is turned off for every table of
-- the test database, so an ANALYZE or VACUUM that autovacuum happens to start in the middle of a run
-- cannot change a plan (a 30-hop variable-length match went from 0.01 s to past the 30 s limit that
-- way, in one of four otherwise identical runs). Scenarios roll back, so the data the statistics
-- describe stays the fixture's.
DO $$
DECLARE r record;
BEGIN
  FOR r IN SELECT c.oid::regclass AS t
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
            WHERE c.relkind IN ('r', 'm')
              AND n.nspname NOT IN ('pg_catalog', 'information_schema') AND n.nspname NOT LIKE 'pg_toast%'
  LOOP
    EXECUTE format('ALTER TABLE %s SET (autovacuum_enabled = false, toast.autovacuum_enabled = false)', r.t);
  END LOOP;
END $$;
ANALYZE;
