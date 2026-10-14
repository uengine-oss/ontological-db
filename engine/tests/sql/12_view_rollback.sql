-- A type view created lazily inside a transaction that rolls back must not
-- leave a cached plan pointing at it. Found loading data over Bolt, where a
-- read-only transaction ended in ROLLBACK and every later query on that
-- connection failed with: relation "og_data.v_N" does not exist.
\set ON_ERROR_STOP on
SET client_min_messages = warning;
SELECT og_create_graph('vb') \g /dev/null
SELECT og_create_type('vb', 'P', 'entity') \g /dev/null
SELECT og_create_type('vb', 'C', 'entity', ARRAY['P']) \g /dev/null
SELECT og_add_property('vb', 'C', 'k', 'string') \g /dev/null

BEGIN;
SELECT og_cypher('vb', 'MATCH (n:C {k:"x"}) RETURN count(n) AS c') \g /dev/null
ROLLBACK;

-- same query, same session: must compile again and rebuild the view
DO $$ BEGIN
    ASSERT (SELECT og_cypher('vb', 'MATCH (n:C {k:"x"}) RETURN count(n) AS c')->>'c') = '0';
END $$;

SELECT og_drop_graph('vb') \g /dev/null
\echo 12_view_rollback: reached the end
