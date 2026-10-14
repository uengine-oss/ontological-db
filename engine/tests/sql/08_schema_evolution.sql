-- Schema evolution — spec 012: Expand → Migrate → Validate → Contract.
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

CREATE FUNCTION pg_temp.col_type(tab text, col text) RETURNS text
LANGUAGE sql AS $$
    SELECT format_type(atttypid, atttypmod) FROM pg_attribute
     WHERE attrelid = tab::regclass AND attname = col AND NOT attisdropped
$$;

SELECT og_create_graph('evo');
SELECT og_create_type('evo', 'Person', 'entity');
SELECT og_add_property('evo', 'Person', 'name', 'string');
SELECT og_add_property('evo', 'Person', 'age', 'string');   -- the mistake to fix
SELECT og_create_type('evo', 'Employee', 'entity', ARRAY['Person']);
SELECT og_create_index('evo', 'Person', 'age');

SELECT og_cypher('evo', 'CREATE (:Person {name: "ann", age: "31"})');
SELECT og_cypher('evo', 'CREATE (:Person {name: "bob", age: "42"})');
SELECT og_cypher('evo', 'CREATE (:Person {name: "cy",  age: "abc"})');
SELECT og_cypher('evo', 'CREATE (:Person {name: "dee"})');
SELECT og_cypher('evo', 'CREATE (:Employee {name: "eve", age: "55"})');

SELECT storage_table AS person_t FROM og_catalog.type WHERE name = 'Person' \gset
SELECT storage_table AS employee_t FROM og_catalog.type WHERE name = 'Employee' \gset
-- psql variables do not reach inside DO bodies; settings do.
SELECT set_config('t.person_t', :'person_t', false), set_config('t.employee_t', :'employee_t', false);

\echo '--- alter_property_type: plan reports what would not survive ---'
SELECT og_schema_plan('evo', '{"kind":"alter_property_type","type":"Person","property":"age","to":"int"}') AS c1 \gset
DO $$
DECLARE r jsonb;
BEGIN
    SELECT report INTO r FROM og_catalog.schema_change WHERE change_id = 1;
    IF (r->>'uncastable')::int <> 1 OR jsonb_array_length(r->'uncastable_samples') <> 1 THEN
        RAISE EXCEPTION 'plan should find exactly one uncastable value (cy/"abc"): %', r;
    END IF;
END $$;
SELECT pg_temp.expect_fail(format('SELECT og_schema_contract(%s)', :c1), '55000', 'OG_SCHEMA_PHASE%');

\echo '--- expand: the new type is enforced on new writes at once ---'
SELECT og_schema_expand(:c1);
SELECT pg_temp.expect_fail($q$ SELECT og_cypher('evo', 'CREATE (:Person {name: "fay", age: "old"})') $q$, '22P02', NULL);
SELECT og_cypher('evo', 'CREATE (:Person {name: "gus", age: "60"})');
-- The dual write is a trigger: raw SQL is held to the new type as well…
SELECT pg_temp.expect_fail(format('UPDATE %s SET p_age = %L WHERE p_name = %L', :'person_t', 'x', 'gus'),
                           '22P02', NULL);
-- …and fills the new column when it changes an already-migrated value.
SELECT format('UPDATE %s SET p_age = %L WHERE p_name = %L', :'person_t', '61', 'gus') AS raw_update \gset
DO $$
BEGIN
    IF (SELECT og_cypher('evo', 'MATCH (p:Person {name: "gus"}) RETURN p.age AS a')->>'a') <> '60' THEN
        RAISE EXCEPTION 'reads during the transition must still see the old column';
    END IF;
END $$;

\echo '--- migrate in batches, resumably ---'
DO $$
DECLARE r jsonb; calls int := 0;
BEGIN
    LOOP
        r := og_schema_migrate(1, 2);
        calls := calls + 1;
        EXIT WHEN (r->>'done')::bool;
        IF calls > 20 THEN RAISE EXCEPTION 'migration does not terminate: %', r; END IF;
    END LOOP;
    IF calls < 3 THEN RAISE EXCEPTION 'batch size 2 over 5 rows took % call(s) — not batching', calls; END IF;
END $$;

-- gus was migrated above; change his age by raw SQL now. Contract must keep it.
:raw_update;

\echo '--- validate names the row that blocks the contract ---'
DO $$
DECLARE r jsonb; cy int8;
BEGIN
    r := og_schema_validate(1);
    cy := (og_cypher('evo', 'MATCH (p:Person {name: "cy"}) RETURN id(p) AS i')->>'i')::int8;
    IF (r->>'violations')::int <> 1 OR (r->'samples'->>0)::int8 <> cy OR r->>'phase' <> 'migrating' THEN
        RAISE EXCEPTION 'validate should block on cy (%): %', cy, r;
    END IF;
END $$;
SELECT pg_temp.expect_fail(format('SELECT og_schema_contract(%s)', :c1), '55000', NULL);

-- Fix the data the ordinary way; the dual write fills the new column.
SELECT og_cypher('evo', 'MATCH (p:Person {name: "cy"}) SET p.age = "7"');
DO $$
BEGIN
    IF og_schema_validate(1)->>'phase' <> 'validated' THEN
        RAISE EXCEPTION 'validation should pass once cy is fixed';
    END IF;
END $$;

\echo '--- contract: the column now is an int, under its own name ---'
SELECT og_schema_contract(:c1);
DO $$
DECLARE n int;
BEGIN
    IF pg_temp.col_type(current_setting('t.person_t'), 'p_age') <> 'integer'
       OR pg_temp.col_type(current_setting('t.employee_t'), 'p_age') <> 'integer' THEN
        RAISE EXCEPTION 'p_age is not an integer after contract';
    END IF;
    IF EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = current_setting('t.person_t')::regclass
                  AND attname LIKE 'p_age\_\_s%' AND NOT attisdropped) THEN
        RAISE EXCEPTION 'the shadow column survived contract';
    END IF;
    IF (og_cypher('evo', 'MATCH (p:Person {name: "gus"}) RETURN p.age AS a')->>'a')::int <> 61 THEN
        RAISE EXCEPTION 'a raw SQL write during the migration was lost at contract';
    END IF;
    -- Numeric, not lexical: "7" < "42" as text would put cy last.
    SELECT count(*) INTO n FROM og_cypher('evo', 'MATCH (p:Person) WHERE p.age > 40 RETURN p.name');
    IF n <> 3 THEN RAISE EXCEPTION 'expected bob, eve, gus over 40, got %', n; END IF;
    -- The index on the old column was rebuilt on the new one.
    IF NOT EXISTS (SELECT 1 FROM pg_indexes WHERE tablename = split_part(current_setting('t.person_t'), '.', 2)
                     AND indexdef LIKE '%(p_age)%') THEN
        RAISE EXCEPTION 'the index on age was not rebuilt';
    END IF;
    IF (SELECT data_type FROM og_catalog.property p JOIN og_catalog.type t USING (type_id)
         WHERE t.name = 'Employee' AND p.name = 'age') <> 'int4' THEN
        RAISE EXCEPTION 'the subtype''s catalog entry still has the old type';
    END IF;
END $$;

\echo '--- the schema record: versions and snapshots ---'
DO $$
DECLARE c record; old jsonb;
BEGIN
    SELECT * INTO c FROM og_catalog.schema_change WHERE change_id = 1;
    IF c.phase <> 'contracted' OR c.to_version IS NULL OR c.to_version <= c.from_version THEN
        RAISE EXCEPTION 'change record wrong: %', row_to_json(c);
    END IF;
    old := og_schema_snapshot('evo', c.from_version);
    IF old->'types'->0->'properties'->'age'->>'type' <> 'text' THEN
        RAISE EXCEPTION 'the snapshot at from_version should show age as text: %', old->'types'->0;
    END IF;
    IF og_schema_snapshot('evo')->'types'->0->'properties'->'age'->>'type' <> 'int4' THEN
        RAISE EXCEPTION 'the current snapshot should show age as int4';
    END IF;
    IF (SELECT count(*) FROM og_schema_changes('evo')) <> 1 THEN
        RAISE EXCEPTION 'og_schema_changes should list one change';
    END IF;
END $$;

\echo '--- rename_property: both names work until contract ---'
SELECT og_schema_plan('evo', '{"kind":"rename_property","type":"Person","property":"name","new_name":"fullName"}') AS c2 \gset
SELECT og_schema_expand(:c2);
SELECT og_cypher('evo', 'CREATE (:Person {fullName: "hal", age: 20})');
DO $$
BEGIN
    IF og_cypher('evo', 'MATCH (p:Person {name: "hal"}) RETURN p.fullName AS f')->>'f' <> 'hal' THEN
        RAISE EXCEPTION 'old and new name should read the same column during a rename';
    END IF;
    IF (SELECT count(*) FROM og_cypher('evo', 'MATCH (p:Employee) RETURN p.fullName')) <> 1 THEN
        RAISE EXCEPTION 'the new name should reach subtypes';
    END IF;
END $$;
SELECT og_schema_validate(:c2);
SELECT og_schema_contract(:c2);
DO $$
BEGIN
    IF og_cypher('evo', 'MATCH (p:Person {fullName: "ann"}) RETURN p.fullName AS f')->>'f' <> 'ann' THEN
        RAISE EXCEPTION 'the new name lost the existing data';
    END IF;
    IF EXISTS (SELECT 1 FROM og_catalog.property WHERE name = 'name') THEN
        RAISE EXCEPTION 'the old name survived contract';
    END IF;
END $$;

\echo '--- set_required: new writes checked at once, old rows filled, then NOT NULL ---'
SELECT og_schema_plan('evo', '{"kind":"set_required","type":"Person","property":"age","default":0}') AS c3 \gset
DO $$
BEGIN
    IF (SELECT (report->>'missing')::int FROM og_catalog.schema_change WHERE change_id = 3) <> 1 THEN
        RAISE EXCEPTION 'plan should find dee without an age';
    END IF;
END $$;
SELECT og_schema_expand(:c3);
SELECT pg_temp.expect_fail($q$ SELECT og_cypher('evo', 'CREATE (:Person {fullName: "ivy"})') $q$, '23514', NULL);
SELECT og_schema_migrate(:c3, 100);
SELECT og_schema_validate(:c3);
SELECT og_schema_contract(:c3);
DO $$
BEGIN
    IF NOT (SELECT attnotnull FROM pg_attribute WHERE attrelid = current_setting('t.person_t')::regclass AND attname = 'p_age') THEN
        RAISE EXCEPTION 'p_age is not NOT NULL after contract';
    END IF;
    IF (SELECT og_cypher('evo', 'MATCH (p:Person {fullName: "dee"}) RETURN p.age AS a')->>'a')::int <> 0 THEN
        RAISE EXCEPTION 'dee was not filled with the default';
    END IF;
END $$;

\echo '--- add_unique: duplicates reported, fixed, then enforced ---'
SELECT og_cypher('evo', 'CREATE (:Employee {fullName: "ann", age: 1})');
SELECT og_schema_plan('evo', '{"kind":"add_unique","type":"Person","property":"fullName"}') AS c4 \gset
SELECT og_schema_expand(:c4);
DO $$
DECLARE r jsonb;
BEGIN
    r := og_schema_validate(4);
    IF (r->>'violations')::int <> 1 OR r->'samples'->>0 <> 'ann' THEN
        RAISE EXCEPTION 'duplicate "ann" across Person and Employee should be reported: %', r;
    END IF;
END $$;
SELECT og_cypher('evo', 'MATCH (p:Employee {fullName: "ann"}) SET p.fullName = "ann2"');
SELECT og_schema_validate(:c4);
SELECT og_schema_contract(:c4);
SELECT pg_temp.expect_fail($q$ SELECT og_cypher('evo', 'CREATE (:Person {fullName: "bob", age: 3})') $q$, '23505', NULL);

\echo '--- abort undoes an expand ---'
SELECT og_schema_plan('evo', '{"kind":"alter_property_type","type":"Person","property":"age","to":"string"}') AS c5 \gset
SELECT og_schema_expand(:c5);
SELECT og_schema_abort(:c5);
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = current_setting('t.person_t')::regclass
                  AND attname LIKE 'p_age\_\_s%' AND NOT attisdropped) THEN
        RAISE EXCEPTION 'abort left the shadow column behind';
    END IF;
    -- And writes stop dual-writing: this would fail against a dropped column.
    PERFORM og_cypher('evo', 'CREATE (:Person {fullName: "jo", age: 9})');
END $$;

\echo '--- drop_property, rename_type ---'
SELECT og_schema_apply('evo', '{"kind":"drop_property","type":"Person","property":"age"}');
DO $$
BEGIN
    IF pg_temp.col_type(current_setting('t.person_t'), 'p_age') IS NOT NULL THEN
        RAISE EXCEPTION 'p_age still exists after drop_property';
    END IF;
END $$;
SELECT pg_temp.expect_fail(
    $q$ SELECT og_schema_plan('evo', '{"kind":"drop_property","type":"Employee","property":"fullName"}') $q$,
    NULL, '%inherited from ''Person''%');
SELECT og_schema_apply('evo', '{"kind":"rename_type","type":"Employee","new_name":"Staff"}');
DO $$
BEGIN
    IF (SELECT count(*) FROM og_cypher('evo', 'MATCH (s:Staff) RETURN s')) <> 2 THEN
        RAISE EXCEPTION 'renamed type does not answer under its new name';
    END IF;
END $$;

\echo '--- alter_role: existing edges are checked, and apply is all-or-nothing ---'
SELECT og_create_type('evo', 'Company', 'entity');
SELECT og_create_type('evo', 'Org', 'entity');
SELECT og_create_type('evo', 'WORKS_AT', 'relation');
SELECT og_add_role('evo', 'WORKS_AT', 'employee', 'Person', 0);
SELECT og_add_role('evo', 'WORKS_AT', 'employer', 'Company', 1);
SELECT og_cypher('evo', 'CREATE (:Company {n: "acme"})');
SELECT og_cypher('evo', 'MATCH (p:Person {fullName: "ann"}), (c:Company) CREATE (p)-[:WORKS_AT]->(c)');
SELECT pg_temp.expect_fail(
    $q$ SELECT og_schema_apply('evo', '{"kind":"alter_role","relation":"WORKS_AT","role":"employer","player":"Org"}') $q$,
    '23514', 'OG_SCHEMA_VIOLATION: 1 row(s)%');
DO $$
BEGIN
    IF (SELECT og_type_name(player_type_id) FROM og_catalog.role WHERE name = 'employer') <> 'Company' THEN
        RAISE EXCEPTION 'a failed apply changed the role';
    END IF;
END $$;

\echo '--- add_parent / remove_parent: a new supertype, no row touched ---'
SELECT og_create_type('evo', 'Agent', 'entity', ARRAY[]::text[], true);
SELECT og_add_property('evo', 'Agent', 'since', 'date');
DO $$
DECLARE before text; after text;
BEGIN
    -- Row versions are the proof: xmin changes on any rewrite.
    SELECT string_agg(xmin::text, ',' ORDER BY id) INTO before FROM og_data.n_1;
    PERFORM og_schema_apply('evo', '{"kind":"add_parent","type":"Person","parent":"Agent"}');
    SELECT string_agg(xmin::text, ',' ORDER BY id) INTO after FROM og_data.n_1;
    IF before <> after THEN RAISE EXCEPTION 'add_parent rewrote rows'; END IF;
    IF (SELECT count(*) FROM og_cypher('evo', 'MATCH (a:Agent) RETURN a')) <>
       (SELECT count(*) FROM og_cypher('evo', 'MATCH (p:Person) RETURN p')) THEN
        RAISE EXCEPTION 'every Person (and Staff) should now answer as an Agent';
    END IF;
    PERFORM og_cypher('evo', 'MATCH (p:Person {fullName: "ann"}) SET p.since = date("2020-01-01")');
END $$;
SELECT pg_temp.expect_fail($q$ SELECT og_schema_plan('evo', '{"kind":"add_parent","type":"Agent","parent":"Staff"}') $q$,
                           NULL, '%cycle%');
SELECT og_schema_apply('evo', '{"kind":"remove_parent","type":"Person","parent":"Agent"}');
DO $$
BEGIN
    IF (SELECT count(*) FROM og_cypher('evo', 'MATCH (a:Agent) RETURN a')) <> 0 THEN
        RAISE EXCEPTION 'Person still answers as Agent after remove_parent';
    END IF;
END $$;

\echo 'schema evolution: all checks passed'
