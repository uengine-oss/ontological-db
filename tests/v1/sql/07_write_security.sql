-- Write-time security context — spec 014.
--
-- Asserts rather than prints: every check raises on failure, and the file runs
-- with ON_ERROR_STOP, so one wrong answer fails the suite. The roles are
-- cluster-wide and survive the test database; they are created if missing and
-- carry no privileges outside it.
\set ON_ERROR_STOP on

DO $$
DECLARE r text;
BEGIN
    FOREACH r IN ARRAY ARRAY['og_t_reader','og_t_writer','og_t_clerk','og_t_agent','og_t_runner'] LOOP
        IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = r) THEN
            EXECUTE format('CREATE ROLE %I', r);
        END IF;
    END LOOP;
END $$;

-- expect_fail(sql, sqlstate or NULL, message LIKE pattern or NULL)
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

SELECT og_create_graph('sec');
SELECT og_create_type('sec', 'Person', 'entity');
SELECT og_add_property('sec', 'Person', 'name', 'string');
SELECT og_add_property('sec', 'Person', 'salary', 'int');
SELECT og_create_type('sec', 'Employee', 'entity', ARRAY['Person']);
SELECT og_create_type('sec', 'Doc', 'entity');
SELECT og_add_property('sec', 'Doc', 'title', 'string');
SELECT og_cypher('sec', 'CREATE (:Person {name: "seed", salary: 1})');
SELECT og_cypher('sec', 'CREATE (:Doc {title: "d1"})');

SELECT og_grant('og_t_reader', 'read');
SELECT og_grant('og_t_writer', 'write');
-- The clerk may create people and set their names — nothing else.
SELECT og_grant_write('og_t_clerk', 'sec', 'Person', ARRAY['create', 'set'], ARRAY['name']);

\echo '--- a reader can read a type whose view has not been built yet ---'
-- Views are created lazily by the first query. Before spec 014 a role that
-- cannot create in og_data got "permission denied for schema og_data" here.
SELECT og_add_property('sec', 'Doc', 'pages', 'int');  -- bumps the version: all views dropped
SET ROLE og_t_reader;
DO $$
DECLARE n int;
BEGIN
    SELECT count(*) INTO n FROM og_cypher('sec', 'MATCH (d:Doc) RETURN d.title');
    IF n <> 1 THEN RAISE EXCEPTION 'reader: expected 1 Doc, got %', n; END IF;
END $$;
RESET ROLE;

\echo '--- a writer without schema rights keeps undeclared properties in __ext ---'
-- ADR-025 said this; the write used to fail on EXECUTE of og_add_property.
SET ROLE og_t_writer;
SELECT og_cypher('sec', 'CREATE (:Person {name: "w", mood: "fine"})');
DO $$
DECLARE got jsonb;
BEGIN
    SELECT og_cypher('sec', 'MATCH (p:Person {name: "w"}) RETURN p.mood AS m') INTO got;
    IF got->>'m' <> 'fine' THEN RAISE EXCEPTION 'undeclared property lost: %', got; END IF;
END $$;
RESET ROLE;
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM og_catalog.property WHERE name = 'mood') THEN
        RAISE EXCEPTION 'a writer promoted a property to a column — that is a schema change';
    END IF;
END $$;

\echo '--- fine-grained grants: what the clerk may and may not do ---'
SET ROLE og_t_clerk;
SELECT og_cypher('sec', 'CREATE (:Person {name: "c1"})');
SELECT og_cypher('sec', 'MATCH (p:Person {name: "c1"}) SET p.name = "c2"');
-- A grant on Person reaches its subtype's table too.
SELECT og_cypher('sec', 'CREATE (:Employee {name: "e1"})');
SELECT pg_temp.expect_fail($q$ SELECT og_cypher('sec', 'CREATE (:Person {name: "c3", salary: 9})') $q$,
                           '42501', 'OG_WRITE_DENIED%salary%');
SELECT pg_temp.expect_fail($q$ SELECT og_cypher('sec', 'MATCH (p:Person {name: "c2"}) SET p.salary = 9') $q$,
                           '42501', 'OG_WRITE_DENIED%salary%');
SELECT pg_temp.expect_fail($q$ SELECT og_cypher('sec', 'MATCH (p:Person {name: "c2"}) SET p.nick = "x"') $q$,
                           '42501', 'OG_WRITE_DENIED%undeclared%');
SELECT pg_temp.expect_fail($q$ SELECT og_cypher('sec', 'MATCH (p:Person {name: "c2"}) DETACH DELETE p') $q$,
                           '42501', 'OG_WRITE_DENIED%delete%');
SELECT pg_temp.expect_fail($q$ SELECT og_cypher('sec', 'CREATE (:Doc {title: "nope"})') $q$,
                           '42501', 'OG_WRITE_DENIED%Doc%');
RESET ROLE;

\echo '--- the same rules hold for raw SQL, which never passes through the engine ---'
SELECT format('%s', storage_table) AS person_table FROM og_catalog.type WHERE name = 'Person' \gset
SELECT format('%s', type_id) AS doc_tid FROM og_catalog.type WHERE name = 'Doc' \gset
SET ROLE og_t_clerk;
SELECT pg_temp.expect_fail(format('UPDATE %s SET p_salary = 100', :'person_table'), '42501', NULL);
SELECT pg_temp.expect_fail(format('DELETE FROM %s', :'person_table'), '42501', NULL);
-- Registry rows for a type the clerk cannot write are refused by row-level security.
SELECT pg_temp.expect_fail(
    format('INSERT INTO og_data.og_node (id, type_id) VALUES (og_make_id(0, %s, 999), %s)', :'doc_tid', :'doc_tid'),
    '42501', '%row-level security%');
RESET ROLE;

\echo '--- reserved properties cannot be written ---'
SELECT pg_temp.expect_fail($q$ SELECT og_cypher('sec', 'CREATE (:Person {name: "r", _type: "Admin"})') $q$,
                           '42501', 'OG_RESERVED_PROPERTY%_type%');
SELECT pg_temp.expect_fail($q$ SELECT og_cypher('sec', 'MATCH (p:Person {name: "seed"}) SET p.__actor = "x"') $q$,
                           '42501', 'OG_RESERVED_PROPERTY%');

\echo '--- system-managed stamps: from the session, not from the row ---'
SELECT og_enable_stamping('sec', 'Person');
SET ROLE og_t_clerk;
SET og.actor = 'agent-7';
SET og.source = 'crm-import';
SELECT og_cypher('sec', 'CREATE (:Person {name: "stamped"})');
RESET og.actor;
RESET og.source;
RESET ROLE;
DO $$
DECLARE m jsonb;
BEGIN
    SELECT og_meta((og_cypher('sec', 'MATCH (p:Person {name: "stamped"}) RETURN id(p) AS i')->>'i')::int8) INTO m;
    IF m->>'created_by' <> 'og_t_clerk' OR m->>'actor' <> 'agent-7' OR m->>'source' <> 'crm-import'
       OR m->>'created_at' IS NULL THEN
        RAISE EXCEPTION 'stamps wrong: %', m;
    END IF;
END $$;
-- A writer holding UPDATE on the whole table still cannot forge them.
SET ROLE og_t_writer;
SELECT format('UPDATE %s SET __created_by = %L, __actor = %L WHERE p_name = %L',
              :'person_table', 'mallory', 'forged', 'stamped') AS forge \gset
:forge;
RESET ROLE;
DO $$
DECLARE m jsonb;
BEGIN
    SELECT og_meta((og_cypher('sec', 'MATCH (p:Person {name: "stamped"}) RETURN id(p) AS i')->>'i')::int8) INTO m;
    IF m->>'created_by' <> 'og_t_clerk' OR m->>'actor' <> 'agent-7' THEN
        RAISE EXCEPTION 'raw SQL forged a stamp: %', m;
    END IF;
END $$;
-- A new subtype of a stamped type is stamped too.
SELECT og_create_type('sec', 'Contractor', 'entity', ARRAY['Person']);
SELECT og_cypher('sec', 'CREATE (:Contractor {name: "k"})');
DO $$
BEGIN
    IF og_meta((og_cypher('sec', 'MATCH (p:Contractor) RETURN id(p) AS i')->>'i')::int8)->>'created_by' IS NULL THEN
        RAISE EXCEPTION 'a subtype created after og_enable_stamping is not stamped';
    END IF;
END $$;

\echo '--- the audit log: who, as the database knows it and as the app claims ---'
DO $$
DECLARE a record;
BEGIN
    SELECT * INTO a FROM og_data.og_audit WHERE query LIKE '%"stamped"})' ORDER BY audit_id DESC LIMIT 1;
    IF a.db_user <> 'og_t_clerk' OR a.actor <> 'agent-7' OR a.source <> 'crm-import' OR a.schema_version IS NULL THEN
        RAISE EXCEPTION 'audit row wrong: db_user=% actor=% source=% version=%',
            a.db_user, a.actor, a.source, a.schema_version;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM og_data.og_audit WHERE db_user = 'og_t_reader') THEN
        RAISE EXCEPTION 'a reader''s query left no audit row';
    END IF;
END $$;
-- Writers cannot edit their own trail.
SET ROLE og_t_writer;
SELECT pg_temp.expect_fail('DELETE FROM og_data.og_audit', '42501', NULL);
SELECT pg_temp.expect_fail('UPDATE og_data.og_audit SET db_user = ''x''', '42501', NULL);
RESET ROLE;

\echo '--- actions: an agent that can do exactly one thing ---'
SELECT og_grant('og_t_agent', 'read');
SELECT og_grant_write('og_t_runner', 'sec', 'Person', ARRAY['create'], ARRAY['name']);
SELECT og_define_action('sec', 'hire',
    'CREATE (p:Person {name: $name}) RETURN p.name AS name',
    'og_t_runner',
    params => '{"name": "string"}',
    precondition => 'MATCH (p:Person {name: $name}) RETURN count(p) = 0 AS ok');
SELECT og_grant_action('og_t_agent', 'sec', 'hire');

SET ROLE og_t_agent;
-- Its own Cypher cannot write…
SELECT pg_temp.expect_fail($q$ SELECT og_cypher('sec', 'CREATE (:Person {name: "rogue"})') $q$, '42501', NULL);
-- …the action can, as the action's owner…
DO $$
DECLARE got jsonb;
BEGIN
    SELECT og_run_action('sec', 'hire', '{"name": "ada"}') INTO got;
    IF got->>'name' <> 'ada' THEN RAISE EXCEPTION 'action returned %', got; END IF;
END $$;
-- …only when its precondition holds…
SELECT pg_temp.expect_fail($q$ SELECT og_run_action('sec', 'hire', '{"name": "ada"}') $q$,
                           '23514', 'OG_ACTION_REJECTED%');
-- …only with the parameters it declares…
SELECT pg_temp.expect_fail($q$ SELECT og_run_action('sec', 'hire', '{"name": "bo", "salary": 5}') $q$,
                           '22023', '%does not take a parameter named ''salary''%');
SELECT pg_temp.expect_fail($q$ SELECT og_run_action('sec', 'hire', '{"name": 7}') $q$, '22023', NULL);
-- …and the executor behind it is not reachable directly.
SELECT pg_temp.expect_fail($q$ SELECT og__action_exec(1, '{"name": "x"}') $q$, '42501', NULL);
RESET ROLE;

SET ROLE og_t_reader;
SELECT pg_temp.expect_fail($q$ SELECT og_run_action('sec', 'hire', '{"name": "eve"}') $q$, '42501', NULL);
RESET ROLE;

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM og_data.og_audit WHERE action = 'hire' AND db_user = 'og_t_runner') THEN
        RAISE EXCEPTION 'the action''s write is not audited under its name';
    END IF;
    IF (SELECT count(*) FROM og_cypher('sec', 'MATCH (p:Person {name: "ada"}) RETURN p')) <> 1 THEN
        RAISE EXCEPTION 'expected exactly one ada';
    END IF;
END $$;

\echo '--- a failed action does not leave its name on later audit rows ---'
SELECT og_define_action('sec', 'raise', 'MATCH (p:Person {name: $name}) SET p.salary = 1', 'og_t_runner',
                        params => '{"name": "string"}');
SELECT og_grant_action('og_t_agent', 'sec', 'raise');
SET ROLE og_t_agent;
-- The runner holds no privilege on salary, so the body fails.
SELECT pg_temp.expect_fail($q$ SELECT og_run_action('sec', 'raise', '{"name": "ada"}') $q$, '42501', NULL);
SELECT og_cypher('sec', 'MATCH (p:Person) RETURN count(p)');
RESET ROLE;
DO $$
BEGIN
    IF (SELECT action FROM og_data.og_audit WHERE db_user = 'og_t_agent' ORDER BY audit_id DESC LIMIT 1) IS NOT NULL THEN
        RAISE EXCEPTION 'the failed action''s name leaked into a later audit row';
    END IF;
END $$;

\echo '--- og.reason is recorded ---'
SET og.reason = 'quarterly review';
SELECT og_cypher('sec', 'MATCH (p:Person) RETURN count(p) AS reasoned');
RESET og.reason;
DO $$
BEGIN
    IF (SELECT reason FROM og_data.og_audit WHERE query LIKE '%AS reasoned' ORDER BY audit_id DESC LIMIT 1)
       IS DISTINCT FROM 'quarterly review' THEN
        RAISE EXCEPTION 'og.reason was not recorded';
    END IF;
END $$;

\echo '--- revoking takes the privilege back ---'
SELECT og_revoke_write('og_t_clerk', 'sec', 'Person', ARRAY['create']);
SET ROLE og_t_clerk;
SELECT pg_temp.expect_fail($q$ SELECT og_cypher('sec', 'CREATE (:Person {name: "late"})') $q$, '42501', NULL);
SELECT og_cypher('sec', 'MATCH (p:Person {name: "c2"}) SET p.name = "c4"');  -- set remains
RESET ROLE;

\echo 'write security: all checks passed'
