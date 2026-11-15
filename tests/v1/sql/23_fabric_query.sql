-- QUERY FABRIC — one read-only SQL statement over the datasources' tables.
--
-- The source is this database again, through a loopback (as in 15): a role that
-- logs in with a password (og_fabric_pw — the test server's pg_hba asks one of it
-- over TCP), because the statement runs as og_source_reader, a role that is not a
-- superuser, and postgres_fdw lets such a role through only with a password.
-- What it must never be: a write, or a way into this database beyond the
-- datasource tables — the catalogs, the engine's tables, pg_* functions.
-- Each refusal is caught by its SQLSTATE and checked; any other error fails the
-- file (no EXPECT_ERROR).
\set ON_ERROR_STOP on

CREATE SCHEMA src23;
CREATE TABLE src23.supplier (supplier_id int PRIMARY KEY, name text, rating numeric(4,2), since date,
                             active boolean, big bigint, score float8, doc jsonb, tags text[]);
INSERT INTO src23.supplier VALUES
    (1, '대성케이블', 4.50, '2020-01-02', true, 9007199254740993, 0.1, '{"a": [1, 2]}', '{x,y}'),
    (2, 'Acme', 3.25, '2021-06-30', false, 7, 'NaN', NULL, NULL),
    (3, 'Nord', NULL, NULL, true, NULL, 2.5, '[]', '{}');
CREATE TABLE src23.big AS SELECT g AS id, 'item-' || g AS label FROM generate_series(1, 3000) g;
-- something in this database that is not a datasource table
CREATE SCHEMA secret23;
CREATE TABLE secret23.notes (t text);
INSERT INTO secret23.notes VALUES ('classified');

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'og_fabric_pw') THEN
        CREATE ROLE og_fabric_pw LOGIN;
    END IF;
END $$;
ALTER ROLE og_fabric_pw PASSWORD 's3cret-pw:1';
GRANT USAGE ON SCHEMA src23 TO og_fabric_pw;
GRANT SELECT ON ALL TABLES IN SCHEMA src23 TO og_fabric_pw;
-- The source account may write supplier: the read-only refusals below are then
-- this engine's, not the source's (with use_remote_estimate the source plans
-- FOR SHARE first, and an account without UPDATE would refuse it there).
GRANT UPDATE ON src23.supplier TO og_fabric_pw;

SELECT og_fabric(format(
  'CREATE DATASOURCE q23 WITH ENGINE = ''postgres'', PARAMETERS = %s',
  jsonb_build_object('host', '127.0.0.1', 'port', current_setting('port'), 'database', current_database(),
                     'schema', 'src23', 'user', 'og_fabric_pw', 'password', 's3cret-pw:1')));

-- What CREATE DATASOURCE left: the reader, granted the datasource's tables and nothing else.
DO $$
DECLARE r record; granted text;
BEGIN
    SELECT rolcanlogin, rolsuper, rolinherit, rolcreaterole, rolcreatedb, rolreplication, rolbypassrls, rolpassword
      INTO r FROM pg_authid WHERE rolname = 'og_source_reader';
    IF NOT FOUND THEN RAISE EXCEPTION 'CREATE DATASOURCE did not create og_source_reader'; END IF;
    IF r.rolcanlogin OR r.rolsuper OR r.rolinherit OR r.rolcreaterole OR r.rolcreatedb OR r.rolreplication
       OR r.rolbypassrls OR r.rolpassword IS NOT NULL THEN
        RAISE EXCEPTION 'og_source_reader is more than a reader: %', r;
    END IF;
    IF EXISTS (SELECT 1 FROM pg_auth_members WHERE member = 'og_source_reader'::regrole) THEN
        RAISE EXCEPTION 'og_source_reader belongs to a role';
    END IF;
    SELECT string_agg(n.nspname || '.' || c.relname, ',' ORDER BY 1) INTO granted
      FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
     WHERE has_table_privilege('og_source_reader', c.oid, 'SELECT')
       AND n.nspname NOT IN ('pg_catalog', 'information_schema');
    IF granted IS DISTINCT FROM 'q23.big,q23.supplier' THEN
        RAISE EXCEPTION 'og_source_reader should read exactly the datasource tables, reads: %', granted;
    END IF;
END $$;

-- The statement: columns and types in order, rows typed.
DO $$
DECLARE got jsonb;
BEGIN
    IF og_cypher_columns('QUERY FABRIC SELECT 1') <> ARRAY['columns','types','rows','row_count','truncated','bytes','elapsed_ms'] THEN
        RAISE EXCEPTION 'QUERY FABRIC columns: %', og_cypher_columns('QUERY FABRIC SELECT 1');
    END IF;
    IF (og_cypher_check('QUERY FABRIC SELECT 1')->>'write')::bool OR NOT (og_cypher_check('QUERY FABRIC SELECT 1')->>'fabric')::bool THEN
        RAISE EXCEPTION 'QUERY FABRIC should be a reading fabric statement: %', og_cypher_check('QUERY FABRIC SELECT 1');
    END IF;
    -- Cypher's own WITH stays Cypher
    IF og_cypher_check('WITH 1 AS x RETURN x') ? 'fabric' THEN RAISE EXCEPTION 'WITH … RETURN taken for a fabric statement'; END IF;

    -- the settings block takes Cypher-map syntax and $parameters; the SQL follows verbatim
    SELECT r INTO got FROM og_fabric(
        'QUERY FABRIC WITH {datasource: $ds} SELECT supplier_id, name, rating, since, active, big, score, doc, tags FROM supplier ORDER BY 1;',
        jsonb_build_object('ds', 'q23')) r;
    IF got->'columns' <> '["supplier_id","name","rating","since","active","big","score","doc","tags"]'
       OR got->'types' <> '["integer","text","numeric","date","boolean","bigint","double precision","jsonb","text[]"]' THEN
        RAISE EXCEPTION 'columns and types: %', got;
    END IF;
    IF got->'rows'->0 <> '[1, "대성케이블", 4.5, "2020-01-02", true, "9007199254740993", 0.1, {"a": [1, 2]}, "{x,y}"]'
       OR got->'rows'->1 <> '[2, "Acme", 3.25, "2021-06-30", false, 7, "NaN", null, null]'
       OR got->'rows'->2 <> '[3, "Nord", null, null, true, null, 2.5, [], "{}"]' THEN
        RAISE EXCEPTION 'typed values: %', got->'rows';
    END IF;
    IF (got->>'row_count')::int <> 3 OR (got->>'truncated')::bool THEN RAISE EXCEPTION 'count: %', got; END IF;

    -- a column name twice, in the order asked; unqualified names find any datasource's table;
    -- an SQL WITH right after the prefix is the SQL's
    SELECT r INTO got FROM og_fabric(
        'QUERY FABRIC WITH b AS (SELECT id FROM big WHERE id < 3) SELECT b.id, s.name, b.id FROM b JOIN q23.supplier s ON s.supplier_id = b.id ORDER BY 1') r;
    IF got->'columns' <> '["id","name","id"]' OR got->'rows' <> '[[1, "대성케이블", 1], [2, "Acme", 2]]' THEN
        RAISE EXCEPTION 'repeated column / SQL WITH: %', got;
    END IF;
END $$;

-- Rows are capped; `truncated` says there were more.
DO $$
DECLARE got jsonb;
BEGIN
    SELECT r INTO got FROM og_fabric('QUERY FABRIC SELECT id FROM q23.big ORDER BY id') r;
    IF jsonb_array_length(got->'rows') <> 1000 OR NOT (got->>'truncated')::bool OR got->'rows'->999->>0 <> '1000' THEN
        RAISE EXCEPTION 'default cap 1000: % rows, truncated %', jsonb_array_length(got->'rows'), got->'truncated';
    END IF;
    SELECT r INTO got FROM og_fabric('QUERY FABRIC WITH {max_rows: 10} SELECT id FROM q23.big ORDER BY id') r;
    IF jsonb_array_length(got->'rows') <> 10 OR NOT (got->>'truncated')::bool THEN RAISE EXCEPTION 'max_rows 10: %', got; END IF;
    SELECT r INTO got FROM og_fabric('QUERY FABRIC SELECT id FROM q23.big ORDER BY id LIMIT 5') r;
    IF jsonb_array_length(got->'rows') <> 5 OR (got->>'truncated')::bool THEN RAISE EXCEPTION 'LIMIT 5: %', got; END IF;
END $$;

-- Refusals, each by its SQLSTATE, which its message carries.
CREATE FUNCTION pg_temp.refused(q text, opts jsonb, want text) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    PERFORM og_fabric_query(q, opts);
    RAISE EXCEPTION 'ran, should have been refused (%): %', want, q USING ERRCODE = 'P0004';
EXCEPTION WHEN assert_failure THEN RAISE;
          WHEN OTHERS THEN
    IF SQLSTATE <> want OR position(format('[SQLSTATE %s', want) IN SQLERRM) = 0 THEN
        RAISE EXCEPTION 'expected SQLSTATE % for %, got %: %', want, q, SQLSTATE, SQLERRM USING ERRCODE = 'P0004';
    END IF;
END $$;

-- the statement check: writes and anything but one SELECT/WITH, before anything runs
SELECT pg_temp.refused('DELETE FROM q23.supplier', '{}', '42601');
SELECT pg_temp.refused('WITH d AS (DELETE FROM q23.supplier RETURNING *) SELECT * FROM d', '{}', '42601');
SELECT pg_temp.refused('SELECT 1; DROP TABLE q23.supplier', '{}', '42601');
SELECT pg_temp.refused('SELECT * INTO q23.copy FROM q23.supplier', '{}', '42601');
SELECT pg_temp.refused('SET ROLE og_fabric_pw', '{}', '42601');
-- what the check lets through still cannot write: the transaction is read only
SELECT pg_temp.refused('SELECT * FROM q23.supplier FOR SHARE', '{}', '25006');
SELECT pg_temp.refused('SELECT set_config(''transaction_read_only'', ''off'', true)', '{}', '25001');
-- nor read past the datasources: not a local table, not the engine's tables, not the catalogs
-- (PUBLIC may read pg_class; a fabric query may not), not a server file
SELECT pg_temp.refused('SELECT * FROM secret23.notes', '{}', '42501');
SELECT pg_temp.refused('SELECT * FROM og_catalog.datasource', '{}', '42501');
SELECT pg_temp.refused('SELECT * FROM og_data.og_audit', '{}', '42501');
SELECT pg_temp.refused('SELECT relname FROM pg_class', '{}', '42501');
SELECT pg_temp.refused('SELECT srvname, umoptions FROM pg_user_mappings', '{}', '42501');
SELECT pg_temp.refused('SELECT table_name FROM information_schema.tables', '{}', '42501');
SELECT pg_temp.refused('SELECT s.name FROM q23.supplier s WHERE s.supplier_id IN (SELECT oid::int FROM pg_roles)', '{}', '42501');
SELECT pg_temp.refused('SELECT pg_read_file(''postgresql.conf'')', '{}', '42501');
SELECT pg_temp.refused('SELECT * FROM pg_ls_dir(''.'')', '{}', '42501');
SELECT pg_temp.refused('SELECT pg_sleep(0)', '{}', '42501');
SELECT pg_temp.refused('SELECT pg_advisory_lock(23)', '{}', '42501');
SELECT pg_temp.refused('SELECT count(*) FROM q23.supplier WHERE pg_backend_pid() > 0', '{}', '42501');
-- a nested query (query_to_xml runs its SQL text before it makes XML, so this holds
-- on a server built without libxml too) is held to the same
SELECT pg_temp.refused('SELECT query_to_xml(''SELECT relname FROM pg_class'', true, true, '''')', '{}', '42501');
-- nor become someone else: the user-id switch is security-restricted
SELECT pg_temp.refused('SELECT set_config(''role'', ''og_fabric_pw'', false)', '{}', '42501');
SELECT pg_temp.refused('SELECT set_config(''session_authorization'', ''og_fabric_pw'', false)', '{}', '42501');
-- nor call the engine from inside (unqualified, its functions are not even on the path)
SELECT pg_temp.refused('SELECT og_fabric(''SHOW DATASOURCES'')', '{}', '42883');
SELECT pg_temp.refused('SELECT * FROM public.og_fabric(''SHOW DATASOURCES'')', '{}', '42501');
SELECT pg_temp.refused('SELECT public.og_statement_timeout()', '{}', '42501');
-- the options
SELECT pg_temp.refused('SELECT 1', '{"datasource": "nowhere"}', '42704');
SELECT pg_temp.refused('SELECT 1', '{"rows": 5}', '22023');
SELECT pg_temp.refused('SELECT 1', '{"max_rows": -1}', '22023');
SELECT pg_temp.refused('SELECT 1', '{"timeout_ms": 999000}', '22023');

-- A name the datasource does not have is the source's own error, positioned in the caller's SQL.
DO $$
BEGIN
    PERFORM og_fabric('QUERY FABRIC WITH {datasource: ''q23''} SELECT nope FROM supplier');
    RAISE EXCEPTION 'ran';
EXCEPTION WHEN undefined_column THEN
    IF SQLERRM NOT LIKE 'column "nope" does not exist [SQLSTATE 42703, position 8]' THEN
        RAISE EXCEPTION 'position: %', SQLERRM;
    END IF;
END $$;

-- The byte cap ends a result before it is built whole.
SELECT pg_temp.refused('SELECT repeat(''x'', 2 * 1024 * 1024) AS huge', '{"max_bytes": 1048576}', '54000');
DO $$
DECLARE got jsonb;
BEGIN
    SELECT r INTO got FROM og_fabric('QUERY FABRIC WITH {max_bytes: 1048576} SELECT id, label FROM q23.big ORDER BY id') r;
    IF jsonb_array_length(got->'rows') <> 1000 THEN RAISE EXCEPTION '1 MB cap on 40 KB: %', got->'row_count'; END IF;
END $$;

-- The time limit is the statement's: og.statement_timeout (over Bolt, the driver's
-- transaction timeout). Cancelled here and at the source.
SET og.statement_timeout = '1s';
DO $$
DECLARE started timestamptz := clock_timestamp();
BEGIN
    BEGIN
        PERFORM og_fabric('QUERY FABRIC SELECT count(*) FROM q23.big a, q23.big b, q23.big c');
        RAISE EXCEPTION 'the cartesian product finished';
    EXCEPTION WHEN query_canceled THEN
        IF SQLERRM NOT LIKE '%statement timeout [SQLSTATE 57014]' THEN RAISE; END IF;
    END;
    IF clock_timestamp() - started > interval '10 seconds' THEN
        RAISE EXCEPTION 'a 1 s limit cancelled only after %', clock_timestamp() - started;
    END IF;
END $$;
RESET og.statement_timeout;
SELECT pg_sleep(0.5);
SELECT 1 / (count(*) = 0)::int AS source_query_gone
  FROM pg_stat_activity
 WHERE datname = current_database() AND pid <> pg_backend_pid() AND state = 'active';
-- the statement sees its limit
SET og.statement_timeout = '5s';
SELECT 1 / ((og_fabric_query('SELECT current_setting(''statement_timeout'') AS t', '{}')->'rows'->0->>0) = '5s')::int
       AS sees_its_limit;
RESET og.statement_timeout;

-- Nothing the statement did outlives it: the role, the read-only mode, a setting made "for the session".
DO $$
DECLARE got jsonb;
BEGIN
    SELECT r INTO got FROM og_fabric('QUERY FABRIC SELECT current_user, set_config(''search_path'', ''x'', false)') r;
    IF got->'rows'->0->>0 <> 'og_source_reader' THEN RAISE EXCEPTION 'ran as %', got->'rows'->0->>0; END IF;
    IF current_user = 'og_source_reader' OR current_setting('transaction_read_only')::bool
       OR current_setting('search_path') = 'x' THEN
        RAISE EXCEPTION 'the statement left its mark: user %, read only %, search_path %',
            current_user, current_setting('transaction_read_only'), current_setting('search_path');
    END IF;
    -- and its checks are off again: this session reads its catalogs and calls pg_* as before
    PERFORM count(*) FROM pg_class WHERE pg_backend_pid() > 0;
END $$;

-- Tables brought in later are granted: by REFRESH DATASOURCE, and — for a table
-- imported some other way — by the next fabric query.
CREATE TABLE src23.dock (dock_no text);
INSERT INTO src23.dock VALUES ('D-1'), ('D-2');
GRANT SELECT ON src23.dock TO og_fabric_pw;
SELECT og_fabric('REFRESH DATASOURCE q23');
SELECT 1 / has_table_privilege('og_source_reader', 'q23.dock', 'SELECT')::int AS refresh_granted;
DROP FOREIGN TABLE q23.dock;
IMPORT FOREIGN SCHEMA src23 LIMIT TO (dock) FROM SERVER og_ds_q23 INTO q23;
SELECT 1 / (NOT has_table_privilege('og_source_reader', 'q23.dock', 'SELECT'))::int AS imported_by_hand;
SELECT 1 / ((og_fabric_query('SELECT count(*) FROM dock', '{"datasource": "q23"}')->'rows'->0->>0)::int = 2)::int
       AS granted_on_first_query;

-- A role that could not become the reader cannot run fabric queries; a member can.
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'og23_nobody') THEN CREATE ROLE og23_nobody; END IF;
END $$;
GRANT EXECUTE ON FUNCTION og_fabric_query(text, jsonb) TO og23_nobody;
GRANT USAGE ON SCHEMA og_catalog TO og23_nobody;
GRANT SELECT ON og_catalog.datasource TO og23_nobody;
SET ROLE og23_nobody;
SELECT pg_temp.refused('SELECT 1', '{}', '42501');
RESET ROLE;
GRANT og_source_reader TO og23_nobody;
SET ROLE og23_nobody;
SELECT 1 / ((og_fabric_query('SELECT count(*) FROM supplier', '{}')->'rows'->0->>0)::int = 3)::int AS member_may;
RESET ROLE;
REVOKE og_source_reader FROM og23_nobody;
REVOKE SELECT ON og_catalog.datasource FROM og23_nobody;
REVOKE USAGE ON SCHEMA og_catalog FROM og23_nobody;
REVOKE EXECUTE ON FUNCTION og_fabric_query(text, jsonb) FROM og23_nobody;

-- An earlier application's login role of the same name is made a reader again.
ALTER ROLE og_source_reader LOGIN INHERIT PASSWORD 'old-app';
ALTER ROLE og_source_reader SET default_transaction_read_only = on;
GRANT og_fabric_pw TO og_source_reader;
SELECT og_source_reader_sync() AS corrected;
DO $$
DECLARE r record;
BEGIN
    SELECT rolcanlogin, rolinherit, rolpassword INTO r FROM pg_authid WHERE rolname = 'og_source_reader';
    IF r.rolcanlogin OR r.rolinherit OR r.rolpassword IS NOT NULL
       OR EXISTS (SELECT 1 FROM pg_auth_members WHERE member = 'og_source_reader'::regrole)
       OR EXISTS (SELECT 1 FROM pg_db_role_setting WHERE setrole = 'og_source_reader'::regrole) THEN
        RAISE EXCEPTION 'og_source_reader was not corrected: %', r;
    END IF;
END $$;

-- Every fabric query is in the audit trail with its SQL.
SELECT 1 / (count(*) > 0)::int AS audited
  FROM og_data.og_audit WHERE lang = 'fabric' AND query LIKE '%QUERY FABRIC SELECT id FROM q23.big ORDER BY id%';

-- A database whose extension script predates QUERY FABRIC has no og_fabric_query();
-- the statement does not need it (only the library is new after an update).
ALTER EXTENSION ontological DROP FUNCTION og_fabric_query(text, jsonb);
DROP FUNCTION og_fabric_query(text, jsonb);
SELECT 1 / ((r->'rows'->0->>0)::int = 3)::int AS without_the_sql_function
  FROM og_fabric('QUERY FABRIC SELECT count(*) FROM q23.supplier') r;

SELECT og_fabric('DROP DATASOURCE q23');
