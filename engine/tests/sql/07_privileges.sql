-- Regressions for the privilege model: three defects that made it unusable and
-- one that made it useless for isolating projects.
--
-- None of these were caught by anything. The suite ran as the owner, and the
-- owner is exactly the role for which every one of them is invisible. So this
-- file does its work behind SET ROLE, which is the only vantage point from
-- which a grant means anything.
--
-- Asserts rather than prints; see 06 for why. Exactly one error is expected, at
-- the very end.
\set ON_ERROR_STOP on

SELECT og_create_graph('alpha');
SELECT og_create_graph('beta');

SELECT og_create_type('alpha', 'Doc', 'entity');
SELECT og_add_property('alpha', 'Doc', 'title', 'string');
SELECT og_create_type('beta', 'Secret', 'entity');
SELECT og_add_property('beta', 'Secret', 'title', 'string');

SELECT og_cypher('alpha', 'CREATE (:Doc {title:"a1"})');
SELECT og_cypher('alpha', 'CREATE (:Doc {title:"a2"})');
SELECT og_cypher('beta',  'CREATE (:Secret {title:"b1"})');

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'og_test_alpha') THEN
        CREATE ROLE og_test_alpha;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'og_test_wide') THEN
        CREATE ROLE og_test_wide;
    END IF;
END $$;

\echo '--- a read grant is enough to run a read ---'
-- Every query appends to og_audit inside the caller's transaction, and that
-- insert used to be a write privilege. A role granted 'read' could not run one
-- statement: it failed on the audit row before it could return its own answer.
SELECT og_grant('og_test_alpha', 'read', 'alpha');

DO $$
DECLARE n int;
BEGIN
    SET LOCAL ROLE og_test_alpha;
    SELECT count(*) INTO n FROM og_cypher('alpha', 'MATCH (d:Doc) RETURN d.title AS t');
    IF n <> 2 THEN
        RAISE EXCEPTION 'scoped read: expected 2 rows from its own graph, got %', n;
    END IF;
    RESET ROLE;
END $$;

\echo '--- a scoped grant does not reach another graph ---'
DO $$
DECLARE n int; denied bool := false;
BEGIN
    SET LOCAL ROLE og_test_alpha;
    BEGIN
        SELECT count(*) INTO n FROM og_cypher('beta', 'MATCH (s:Secret) RETURN s.title AS t');
    EXCEPTION WHEN insufficient_privilege THEN
        denied := true;
    END;
    RESET ROLE;
    IF NOT denied THEN
        RAISE EXCEPTION 'graph isolation: alpha''s role read beta, got % rows', n;
    END IF;
END $$;

\echo '--- a new label does not re-open the boundary ---'
-- Recorded grants are replayed onto storage created later, because a type made
-- tomorrow needs the grant issued today. Replaying every recorded grant onto
-- every new table is what made scoping decay: a role confined to one project
-- stayed confined only until the next label was created anywhere.
SELECT og_create_type('beta', 'Later', 'entity');
SELECT og_add_property('beta', 'Later', 'title', 'string');
SELECT og_cypher('beta', 'CREATE (:Later {title:"b2"})');

DO $$
DECLARE n int; denied bool := false;
BEGIN
    SET LOCAL ROLE og_test_alpha;
    BEGIN
        SELECT count(*) INTO n FROM og_cypher('beta', 'MATCH (l:Later) RETURN l.title AS t');
    EXCEPTION WHEN insufficient_privilege THEN
        denied := true;
    END;
    RESET ROLE;
    IF NOT denied THEN
        RAISE EXCEPTION 'graph isolation decayed on a new label: got % rows', n;
    END IF;
END $$;

\echo '--- creating a label still works while a grant is on record ---'
-- The replay quoted an already-quoted name, and the failed GRANT rolled back
-- the transaction that was creating the type. With any grant recorded, no new
-- label could be created at all.
SELECT og_create_type('alpha', 'Fresh', 'entity');
SELECT og_add_property('alpha', 'Fresh', 'title', 'string');
SELECT og_cypher('alpha', 'CREATE (:Fresh {title:"a3"})');

DO $$
DECLARE n int;
BEGIN
    SET LOCAL ROLE og_test_alpha;
    SELECT count(*) INTO n FROM og_cypher('alpha', 'MATCH (f:Fresh) RETURN f.title AS t');
    IF n <> 1 THEN
        RAISE EXCEPTION 'new label in its own graph: expected 1 row, got %', n;
    END IF;
    RESET ROLE;
END $$;

\echo '--- a reader can read a label whose view was dropped ---'
-- Type views are built on first mention and dropped wholesale when the schema
-- changes, which put CREATE ON SCHEMA og_data on the read path. A read-only
-- role could not build one, so it broke after every schema change and stayed
-- broken until someone with rights happened to run the query.
DO $$
DECLARE v text;
BEGIN
    FOR v IN
        SELECT 'og_data.' || quote_ident(c.relname) FROM pg_class c
          JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = 'og_data' AND c.relkind = 'v'
           AND (c.relname LIKE 'v\_%' OR c.relname LIKE 've\_%')
    LOOP
        EXECUTE format('DROP VIEW IF EXISTS %s CASCADE', v);
    END LOOP;
END $$;

DO $$
DECLARE n int;
BEGIN
    SET LOCAL ROLE og_test_alpha;
    SELECT count(*) INTO n FROM og_cypher('alpha', 'MATCH (d:Doc) RETURN d.title AS t');
    IF n <> 2 THEN
        RAISE EXCEPTION 'read after the views were dropped: expected 2 rows, got %', n;
    END IF;
    RESET ROLE;
END $$;

\echo '--- an unscoped grant still covers every graph ---'
SELECT og_grant('og_test_wide', 'read', '*');

DO $$
DECLARE a int; b int;
BEGIN
    SET LOCAL ROLE og_test_wide;
    SELECT count(*) INTO a FROM og_cypher('alpha', 'MATCH (d:Doc) RETURN d.title AS t');
    SELECT count(*) INTO b FROM og_cypher('beta',  'MATCH (s:Secret) RETURN s.title AS t');
    RESET ROLE;
    IF a <> 2 OR b <> 1 THEN
        RAISE EXCEPTION 'unscoped grant: expected 2 and 1, got % and %', a, b;
    END IF;
END $$;

\echo '--- a scoped revoke takes back one project and leaves the rest ---'
SELECT og_grant('og_test_alpha', 'read', 'beta');
SELECT og_revoke('og_test_alpha', 'beta');

DO $$
DECLARE n int; denied bool := false;
BEGIN
    SET LOCAL ROLE og_test_alpha;
    BEGIN
        SELECT count(*) INTO n FROM og_cypher('beta', 'MATCH (s:Secret) RETURN s.title AS t');
    EXCEPTION WHEN insufficient_privilege THEN
        denied := true;
    END;
    IF NOT denied THEN
        RESET ROLE;
        RAISE EXCEPTION 'scoped revoke left beta readable, got % rows', n;
    END IF;
    SELECT count(*) INTO n FROM og_cypher('alpha', 'MATCH (d:Doc) RETURN d.title AS t');
    RESET ROLE;
    IF n <> 2 THEN
        RAISE EXCEPTION 'scoped revoke took alpha away too, got % rows', n;
    END IF;
END $$;

\echo '--- og_grant refuses a graph that does not exist ---'
\echo 'OG_TEST_END'

-- EXPECT_ERROR: a typo must not record a grant that covers nothing.
SELECT og_grant('og_test_alpha', 'read', 'no_such_graph');
