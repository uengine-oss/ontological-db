-- Bitemporal types — spec 013.
--
-- One small history, then the same Cypher asked at different instants. Valid
-- time is set explicitly with og.valid_from; system time is whatever the clock
-- said, captured between transactions with \gset.
--
-- Asserts rather than prints; ON_ERROR_STOP makes any failed check fail the
-- suite.
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

-- ann's salary at the session's instant, or NULL if ann did not exist then.
CREATE FUNCTION pg_temp.salary() RETURNS int LANGUAGE sql AS $$
    SELECT (og_cypher('tmp', 'MATCH (p:Person {name: "ann"}) RETURN p.salary AS s')->>'s')::int
$$;
-- Where ann worked at the session's instant, and as what.
CREATE FUNCTION pg_temp.job() RETURNS text LANGUAGE sql AS $$
    SELECT string_agg((r->>'c') || '/' || (r->>'t'), ',')
      FROM og_cypher('tmp', 'MATCH (p:Person {name: "ann"})-[w:WORKS_AT]->(c:Company) RETURN c.name AS c, w.title AS t') r
$$;
CREATE FUNCTION pg_temp.check(what text, got anyelement, want anyelement) RETURNS void
LANGUAGE plpgsql AS $$
BEGIN
    IF got IS DISTINCT FROM want THEN
        RAISE EXCEPTION '% (valid_at=%, system_at=%): expected %, got %', what,
            current_setting('og.valid_at', true), current_setting('og.system_at', true), want, got;
    END IF;
END $$;

SELECT og_create_graph('tmp');
SELECT og_create_type('tmp', 'Person', 'entity');
SELECT og_add_property('tmp', 'Person', 'name', 'string');
SELECT og_add_property('tmp', 'Person', 'salary', 'int');
SELECT og_create_type('tmp', 'Company', 'entity');
SELECT og_add_property('tmp', 'Company', 'name', 'string');
SELECT og_create_type('tmp', 'WORKS_AT', 'relation');
SELECT og_add_property('tmp', 'WORKS_AT', 'title', 'string');
SELECT og_enable_temporal('tmp', 'Person');
SELECT og_enable_temporal('tmp', 'Company');
SELECT og_enable_temporal('tmp', 'WORKS_AT');

\echo '--- history: hired in 2020, raised from 2023, raise corrected, left mid-2024 ---'
SET og.valid_from = '2020-01-01';
SELECT og_cypher('tmp', 'CREATE (:Person {name: "ann", salary: 100})');
SELECT og_cypher('tmp', 'CREATE (:Company {name: "acme"})');
SELECT og_cypher('tmp', 'MATCH (p:Person {name: "ann"}), (c:Company) CREATE (p)-[:WORKS_AT {title: "eng"}]->(c)');
RESET og.valid_from;
SELECT pg_sleep(0.01);
SELECT now() AS s1 \gset
SELECT pg_sleep(0.01);

SET og.valid_from = '2023-01-01';
SELECT og_cypher('tmp', 'MATCH (p:Person {name: "ann"}) SET p.salary = 200');
RESET og.valid_from;
SELECT pg_sleep(0.01);
SELECT now() AS s2 \gset
SELECT pg_sleep(0.01);

-- A correction: it was 250 all along. '-infinity' says "not a change in the
-- world, a change in what we know".
SET og.valid_from = '-infinity';
SELECT og_cypher('tmp', 'MATCH (p:Person {name: "ann"}) SET p.salary = 250');
RESET og.valid_from;
SELECT pg_sleep(0.01);
SELECT now() AS s3 \gset
SELECT pg_sleep(0.01);

SET og.valid_from = '2024-06-01';
SELECT og_cypher('tmp', 'MATCH (:Person {name: "ann"})-[w:WORKS_AT]->() DELETE w');
RESET og.valid_from;

\echo '--- now ---'
SELECT pg_temp.check('current salary', pg_temp.salary(), 250);
SELECT pg_temp.check('current job', pg_temp.job(), NULL::text);

\echo '--- valid time: what was true then, as we know it now ---'
SET og.valid_at = '2019-06-01';
SELECT pg_temp.check('before hire', pg_temp.salary(), NULL::int);
SET og.valid_at = '2021-06-01';
SELECT pg_temp.check('2021 salary', pg_temp.salary(), 100);
SELECT pg_temp.check('2021 job', pg_temp.job(), 'acme/eng');
SET og.valid_at = '2023-06-01';
SELECT pg_temp.check('2023 salary, corrected', pg_temp.salary(), 250);
SET og.valid_at = '2024-01-01';
SELECT pg_temp.check('job before leaving', pg_temp.job(), 'acme/eng');
SET og.valid_at = '2025-01-01';
SELECT pg_temp.check('job after leaving', pg_temp.job(), NULL::text);

\echo '--- system time: what we believed then ---'
SET og.valid_at = '2023-06-01';
SELECT set_config('og.system_at', :'s2', false);
SELECT pg_temp.check('2023 salary as known before the correction', pg_temp.salary(), 200);
SELECT set_config('og.system_at', :'s1', false);
SELECT pg_temp.check('2023 salary as known before the raise was recorded', pg_temp.salary(), 100);
SET og.valid_at = '2025-01-01';
SELECT set_config('og.system_at', :'s3', false);
SELECT pg_temp.check('2025 job as known before the departure was recorded', pg_temp.job(), 'acme/eng');
RESET og.valid_at;
RESET og.system_at;

\echo '--- the compiled SQL does not depend on the instant ---'
DO $$
DECLARE a text; b text;
BEGIN
    a := og_cypher_sql('tmp', 'MATCH (p:Person {name: "ann"}) RETURN p.salary AS s');
    PERFORM set_config('og.valid_at', '2021-06-01', false);
    b := og_cypher_sql('tmp', 'MATCH (p:Person {name: "ann"}) RETURN p.salary AS s');
    PERFORM set_config('og.valid_at', '', false);
    IF a <> b THEN RAISE EXCEPTION 'the instant leaked into the compiled SQL'; END IF;
END $$;

\echo '--- deletion: gone now, there before ---'
SELECT og_cypher('tmp', 'CREATE (:Person {name: "bob", salary: 1})');
SELECT pg_sleep(0.01);
SELECT now() AS s4 \gset
SELECT pg_sleep(0.01);
SELECT og_cypher('tmp', 'MATCH (p:Person {name: "bob"}) DETACH DELETE p');
DO $$
BEGIN
    IF (SELECT count(*) FROM og_cypher('tmp', 'MATCH (p:Person {name: "bob"}) RETURN p')) <> 0 THEN
        RAISE EXCEPTION 'a deleted node is still current';
    END IF;
END $$;
SELECT set_config('og.system_at', :'s4', false);
DO $$
BEGIN
    IF (SELECT count(*) FROM og_cypher('tmp', 'MATCH (p:Person {name: "bob"}) RETURN p')) <> 1 THEN
        RAISE EXCEPTION 'a deleted node is missing from the past';
    END IF;
END $$;
RESET og.system_at;

\echo '--- shapes that only see the present refuse an instant instead of lying ---'
SET og.valid_at = '2021-06-01';
SELECT pg_temp.expect_fail($q$ SELECT og_cypher('tmp', 'MATCH (n) RETURN count(n)') $q$,
                           '0A000', 'OG_TEMPORAL_UNSUPPORTED%unlabelled%');
SELECT pg_temp.expect_fail($q$ SELECT og_cypher('tmp', 'MATCH (p:Person)-[:WORKS_AT*1..2]->(c:Company) RETURN c') $q$,
                           '0A000', 'OG_TEMPORAL_UNSUPPORTED%variable-length%');
SELECT pg_temp.expect_fail($q$ SELECT og_typeql('tmp', 'match $p isa Person;') $q$,
                           NULL, '%OG_TEMPORAL_UNSUPPORTED%TypeQL%');
RESET og.valid_at;
-- Without an instant, the same queries run.
SELECT og_cypher('tmp', 'MATCH (n) RETURN count(n) AS n');
SELECT og_cypher('tmp', 'MATCH (p:Person)-[:WORKS_AT*1..2]->(c:Company) RETURN c');

\echo '--- raw SQL is versioned the same way ---'
SELECT storage_table AS person_t FROM og_catalog.type WHERE name = 'Person' \gset
SELECT pg_sleep(0.01);
SELECT now() AS s5 \gset
SELECT pg_sleep(0.01);
UPDATE og_data.n_1 SET p_salary = 300 WHERE p_name = 'ann';
SELECT pg_temp.check('after raw UPDATE', pg_temp.salary(), 300);
SELECT set_config('og.system_at', :'s5', false);
SELECT pg_temp.check('before raw UPDATE', pg_temp.salary(), 250);
RESET og.system_at;

\echo '--- writers cannot touch history; their writes are still captured ---'
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'og_t_writer') THEN CREATE ROLE og_t_writer; END IF;
END $$;
SELECT og_grant('og_t_writer', 'write');
SET ROLE og_t_writer;
SELECT og_cypher('tmp', 'MATCH (p:Person {name: "ann"}) SET p.salary = 310');
SELECT pg_temp.expect_fail('DELETE FROM og_data.h_1', '42501', NULL);
SELECT pg_temp.expect_fail('INSERT INTO og_data.h_1 (id, __sys_to, __op) VALUES (1, now(), ''u'')', '42501', NULL);
RESET ROLE;
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM og_data.h_1 WHERE p_salary = 300 AND __updated_by = current_user) THEN
        RAISE EXCEPTION 'the superseded row is missing from history';
    END IF;
    IF (og_meta((og_cypher('tmp', 'MATCH (p:Person {name: "ann"}) RETURN id(p) AS i')->>'i')::int8)->>'updated_by')
       <> 'og_t_writer' THEN
        RAISE EXCEPTION 'the writer''s update is not stamped with the writer';
    END IF;
END $$;

\echo '--- schema changes reach history; subtypes inherit versioning ---'
SELECT og_add_property('tmp', 'Person', 'email', 'string');
SELECT og_create_type('tmp', 'Manager', 'entity', ARRAY['Person']);
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = 'og_data.h_1'::regclass AND attname = 'p_email') THEN
        RAISE EXCEPTION 'history table did not gain the new column';
    END IF;
    IF to_regclass('og_data.h_' || og_type_id('tmp', 'Manager')) IS NULL THEN
        RAISE EXCEPTION 'a subtype of a temporal type has no history table';
    END IF;
    IF NOT ('Person' = ANY (og_temporal_types('tmp'))) THEN
        RAISE EXCEPTION 'og_temporal_types does not list Person';
    END IF;
END $$;

\echo '--- a change dated in the future does not hide what holds until then ---'
SELECT og_cypher('tmp', 'CREATE (:Person {name: "cal", salary: 10})');
SELECT set_config('og.valid_from', (now() + interval '30 days')::text, false);
SELECT og_cypher('tmp', 'MATCH (p:Person {name: "cal"}) SET p.salary = 20');
RESET og.valid_from;
DO $$
DECLARE q text := 'MATCH (p:Person {name: "cal"}) RETURN p.salary AS s';
BEGIN
    IF (SELECT count(*) FROM og_cypher('tmp', q)) <> 1
       OR (SELECT (r->>'s')::int FROM og_cypher('tmp', q) r) <> 10 THEN
        RAISE EXCEPTION 'until the future change takes effect, cal earns 10 — got %',
            (SELECT jsonb_agg(r) FROM og_cypher('tmp', q) r);
    END IF;
    PERFORM set_config('og.valid_at', (now() + interval '60 days')::text, false);
    IF (og_cypher('tmp', q)->>'s')::int <> 20 THEN
        RAISE EXCEPTION 'after the change takes effect, cal earns 20';
    END IF;
    PERFORM set_config('og.valid_at', '', false);
END $$;

\echo 'bitemporal: all checks passed'
