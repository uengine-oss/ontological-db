-- Specs 012–014: the paths 07–09 do not reach.
--
-- Each block covers one behaviour that was implemented but not asserted when
-- the specs were written (see the unchecked items in their tasks.md).
-- Asserts rather than prints; ON_ERROR_STOP makes any failed check fail.
\set ON_ERROR_STOP on

CREATE FUNCTION pg_temp.expect_fail(q text, state text, pattern text) RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    failed bool := false;
    st text;
    msg text;
BEGIN
    BEGIN
        EXECUTE q;
    EXCEPTION WHEN others THEN
        failed := true; st := SQLSTATE; msg := SQLERRM;
    END;
    IF NOT failed THEN
        RAISE EXCEPTION 'expected to fail, but it succeeded: %', q;
    END IF;
    IF state IS NOT NULL AND st <> state THEN
        RAISE EXCEPTION 'expected SQLSTATE % from [%], got % (%)', state, q, st, msg;
    END IF;
    IF pattern IS NOT NULL AND msg NOT LIKE pattern THEN
        RAISE EXCEPTION 'expected message like "%" from [%], got "%"', pattern, q, msg;
    END IF;
END $$;

SELECT og_create_graph('cov');
SELECT og_create_type('cov', 'P', 'entity');
SELECT og_add_property('cov', 'P', 'name', 'string');
SELECT og_add_property('cov', 'P', 'n', 'string');
SELECT og_create_type('cov', 'Q', 'entity');
SELECT og_create_type('cov', 'R', 'relation');
SELECT og_add_role('cov', 'R', 'from', 'P', 0);
SELECT og_add_role('cov', 'R', 'to', 'P', 1);

\echo '--- 012: schema version bumps on index creation; required counts relation instances ---'
DO $$
DECLARE v int8;
BEGIN
    SELECT last_value INTO v FROM og_catalog.schema_version_seq;
    PERFORM og_create_index('cov', 'P', 'name');
    IF (SELECT last_value FROM og_catalog.schema_version_seq) <= v THEN
        RAISE EXCEPTION 'og_create_index did not bump the schema version';
    END IF;
END $$;
SELECT og_cypher('cov', 'CREATE (:P {name: "a", n: "1"})-[:R]->(:P {name: "b", n: "2"})');
SELECT pg_temp.expect_fail($q$ SELECT og_add_property('cov', 'R', 'weight', 'int', true) $q$,
                           NULL, '%1 existing instance(s)%');

\echo '--- 012: og_schema_snapshot by instant ---'
DO $$
DECLARE before timestamptz := clock_timestamp();
BEGIN
    PERFORM pg_sleep(0.01);
    IF og_schema_snapshot('cov', at => before) IS NULL THEN
        RAISE EXCEPTION 'no snapshot at an instant after the graph existed';
    END IF;
    IF og_schema_snapshot('cov', at => '2000-01-01') IS NOT NULL THEN
        RAISE EXCEPTION 'a snapshot before the graph existed';
    END IF;
END $$;

\echo '--- 012: alter_role succeeds when the data agrees ---'
SELECT og_create_type('cov', 'P2', 'entity', ARRAY['P']);
SELECT og_schema_apply('cov', '{"kind":"alter_role","relation":"R","role":"to","player":"P","card_max":5}');
DO $$
BEGIN
    IF (SELECT card_max FROM og_catalog.role WHERE name = 'to') <> 5 THEN
        RAISE EXCEPTION 'alter_role did not update the cardinality';
    END IF;
END $$;

\echo '--- 012 × 013: a type change reaches the history table ---'
SELECT og_enable_temporal('cov', 'P');
SELECT og_cypher('cov', 'MATCH (p:P {name: "a"}) SET p.n = "10"');   -- history now holds n = "1"
SELECT og_schema_apply('cov', '{"kind":"alter_property_type","type":"P","property":"n","to":"int"}');
DO $$
DECLARE h text := 'og_data.h_' || og_type_id('cov', 'P');
BEGIN
    IF (SELECT format_type(atttypid, atttypmod) FROM pg_attribute
         WHERE attrelid = h::regclass AND attname = 'p_n' AND NOT attisdropped) <> 'integer' THEN
        RAISE EXCEPTION 'the history column was not retyped';
    END IF;
END $$;

\echo '--- 013: og.valid_to, and a change dated after the end of validity ---'
SET og.valid_to = '2030-01-01';
SELECT og_cypher('cov', 'CREATE (:P {name: "temp"})');
RESET og.valid_to;
DO $$
BEGIN
    IF (SELECT upper(__valid) FROM og_data.n_1 WHERE p_name = 'temp') <> '2030-01-01'::timestamptz THEN
        RAISE EXCEPTION 'og.valid_to was not applied';
    END IF;
END $$;
SET og.valid_from = '2031-01-01';
SELECT pg_temp.expect_fail($q$ SELECT og_cypher('cov', 'MATCH (p:P {name: "temp"}) SET p.name = "x"') $q$,
                           '22023', 'OG_TEMPORAL_RANGE%');
RESET og.valid_from;

\echo '--- 013: a relationship variable without one type refuses an instant ---'
SELECT og_enable_temporal('cov', 'R');
SET og.valid_at = '2026-01-01';
SELECT pg_temp.expect_fail($q$ SELECT og_cypher('cov', 'MATCH (a:P)-[r]->(b:P) RETURN r') $q$,
                           '0A000', '%relationship variable%');
RESET og.valid_at;

\echo '--- 008 og_as_of: a deleted entity has no state after its deletion ---'
SELECT og_create_type('cov', 'H', 'entity');
SELECT og_add_property('cov', 'H', 'v', 'int');
SELECT og_enable_history('cov', 'H');
SELECT og_cypher('cov', 'CREATE (:H {v: 1})');
SELECT set_config('t.hid', og_cypher('cov', 'MATCH (h:H) RETURN id(h) AS i')->>'i', false);
BEGIN;
SELECT og_cypher('cov', 'MATCH (h:H) SET h.v = 2');
SELECT og_cypher('cov', 'MATCH (h:H) SET h.v = 3');   -- same transaction, same now()
COMMIT;
DO $$
DECLARE id int8 := (og_cypher('cov', 'MATCH (h:H) RETURN id(h) AS i')->>'i')::int8;
BEGIN
    IF (og_as_of(id, now())->>'p_v')::int <> 3 THEN
        RAISE EXCEPTION 'og_as_of should break a same-transaction tie by order: got %', og_as_of(id, now());
    END IF;
END $$;
SELECT og_cypher('cov', 'MATCH (h:H) DETACH DELETE h');
DO $$
BEGIN
    IF og_as_of(current_setting('t.hid')::int8, now()) IS NOT NULL
       AND og_as_of(current_setting('t.hid')::int8, now()) <> 'null'::jsonb THEN
        RAISE EXCEPTION 'a deleted entity still has a state: %', og_as_of(current_setting('t.hid')::int8, now());
    END IF;
END $$;

\echo '--- 014: edge registry and adjacency refuse a role without the relation grant ---'
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'og_t_clerk') THEN CREATE ROLE og_t_clerk; END IF;
END $$;
SELECT og_grant_write('og_t_clerk', 'cov', 'Q', ARRAY['create']);
SELECT format('INSERT INTO og_data.og_edge (id, type_id, src, dst) VALUES (og_make_id(0, %s, 99), %s, 1, 2)',
              og_type_id('cov', 'R'), og_type_id('cov', 'R')) AS edge_ins \gset
SELECT format('INSERT INTO og_data.og_adj (src, etype, dir, seq, n, nbr, eid) VALUES (1, %s, ''o'', 9, 1, ''{2}'', ''{3}'')',
              og_type_id('cov', 'R')) AS adj_ins \gset
SET ROLE og_t_clerk;
SELECT og_cypher('cov', 'CREATE (:Q)');   -- what it was granted works
SELECT pg_temp.expect_fail(:'edge_ins', '42501', '%row-level security%');
SELECT pg_temp.expect_fail(:'adj_ins', '42501', '%row-level security%');
RESET ROLE;

\echo '--- 012 × 014: a property-scoped writer keeps writing during a type change ---'
-- The shadow column is filled by a trigger; PostgreSQL checks column
-- privileges only on the columns a statement names, so no grant on the shadow
-- is needed.
SELECT og_add_property('cov', 'Q', 'score', 'string');
SELECT og_grant_write('og_t_clerk', 'cov', 'Q', ARRAY['create', 'set'], ARRAY['score']);
SELECT og_schema_plan('cov', '{"kind":"alter_property_type","type":"Q","property":"score","to":"int"}') AS sc \gset
SELECT og_schema_expand(:sc);
SET ROLE og_t_clerk;
SELECT og_cypher('cov', 'CREATE (:Q {score: "5"})');
SELECT og_cypher('cov', 'MATCH (q:Q {score: "5"}) SET q.score = "6"');
RESET ROLE;
SELECT og_schema_migrate(:sc);
SELECT og_schema_validate(:sc);
SELECT og_schema_contract(:sc);
DO $$
BEGIN
    IF (og_cypher('cov', 'MATCH (q:Q) WHERE q.score = 6 RETURN count(q) AS c')->>'c')::int <> 1 THEN
        RAISE EXCEPTION 'the scoped writer''s value did not survive the type change';
    END IF;
END $$;

\echo '--- 014: revoking and dropping an action ---'
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'og_t_runner') THEN CREATE ROLE og_t_runner; END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'og_t_agent') THEN CREATE ROLE og_t_agent; END IF;
END $$;
SELECT og_grant('og_t_agent', 'read');
SELECT og_grant('og_t_runner', 'read');
SELECT og_define_action('cov', 'count_p', 'MATCH (p:P) RETURN count(p) AS c', 'og_t_runner');
SELECT og_grant_action('og_t_agent', 'cov', 'count_p');
SET ROLE og_t_agent;
SELECT og_run_action('cov', 'count_p');
RESET ROLE;
SELECT og_revoke_action('og_t_agent', 'cov', 'count_p');
SET ROLE og_t_agent;
SELECT pg_temp.expect_fail($q$ SELECT og_run_action('cov', 'count_p') $q$, '42501', NULL);
RESET ROLE;
SELECT og_drop_action('cov', 'count_p');
SELECT pg_temp.expect_fail($q$ SELECT og_run_action('cov', 'count_p') $q$, NULL, '%no action named%');

\echo 'coverage 012-014: all checks passed'
