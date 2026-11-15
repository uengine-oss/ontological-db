-- ontological.fabric_inline_passwords: whether a datasource may store a password
-- given inline (`password: …`), which the user mapping then holds in clear.
--
--   allow  (default) — stored, with the warning in the result (0013);
--   refuse — the statement fails (42501) and asks for a credentials reference;
--            nothing is created or changed. What a host that keeps the secrets
--            itself passes at server start: -c ontological.fabric_inline_passwords=refuse.
--
-- A reference, or an empty password, is never refused. The setting is
-- superuser-only: another role can neither SET it nor carry a value of its own
-- across the moment it is defined.
-- No EXPECT_ERROR here: any error fails the file.
\set ON_ERROR_STOP on

CREATE SCHEMA src29;
CREATE TABLE src29.tradeitems (item_code text PRIMARY KEY, title text);
INSERT INTO src29.tradeitems VALUES ('A1', 'lamp');
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'og_fabric_pw') THEN
        CREATE ROLE og_fabric_pw LOGIN;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'og_fab29_plain') THEN
        CREATE ROLE og_fab29_plain LOGIN;
    END IF;
END $$;
ALTER ROLE og_fabric_pw PASSWORD 'pw29-test-only';
GRANT USAGE ON SCHEMA src29 TO og_fabric_pw;
GRANT SELECT ON ALL TABLES IN SCHEMA src29 TO og_fabric_pw;

SELECT set_config('ontological.fabric_credentials_dir', current_setting('data_directory'), false);
SELECT current_setting('data_directory') AS datadir \gset
\setenv OG_CRED_DIR :datadir
\! umask 077 && printf '%s\n' '*:*:*:og_fabric_pw:pw29-test-only' > "$OG_CRED_DIR/og_ds_ref29.pgpass" && chmod 600 "$OG_CRED_DIR/og_ds_ref29.pgpass"

DO $$
DECLARE got jsonb; failed bool; n int; params jsonb;
BEGIN
    params := jsonb_build_object('host', '127.0.0.1', 'port', current_setting('port'), 'database', current_database(),
                                 'schema', 'src29', 'user', 'og_fabric_pw');

    -- allow (the default): stored, with the warning.
    SELECT og_fabric(format('CREATE DATASOURCE inl29 WITH ENGINE = ''postgres'', PARAMETERS = %s',
                            params || '{"password": "pw29-test-only"}')) INTO got;
    IF got->>'result' NOT LIKE '%the password is stored inline%' THEN RAISE EXCEPTION 'allow: no warning: %', got; END IF;
    SELECT count(*) INTO n FROM inl29.tradeitems;
    IF n <> 1 THEN RAISE EXCEPTION 'allow: the datasource does not read'; END IF;

    -- refuse: CREATE with a password fails and leaves nothing behind.
    SET ontological.fabric_inline_passwords = refuse;
    failed := false;
    BEGIN
        PERFORM og_fabric(format('CREATE DATASOURCE ref29x WITH ENGINE = ''postgres'', PARAMETERS = %s',
                                 params || '{"password": "pw29-test-only"}'));
    EXCEPTION WHEN insufficient_privilege THEN
        failed := SQLERRM LIKE '%a password given inline is refused here%credentials reference%';
        -- the message never repeats the password
        IF SQLERRM LIKE '%pw29-test-only%' THEN RAISE EXCEPTION 'the refusal quoted the password'; END IF;
    END;
    IF NOT failed THEN RAISE EXCEPTION 'refuse: an inline password was stored'; END IF;
    IF EXISTS (SELECT 1 FROM og_catalog.datasource WHERE name = 'ref29x')
       OR EXISTS (SELECT 1 FROM pg_foreign_server WHERE srvname = 'og_ds_ref29x') THEN
        RAISE EXCEPTION 'refuse: something was created';
    END IF;
    -- … and so does the direct function.
    failed := false;
    BEGIN
        PERFORM og_create_datasource('ref29y', 'postgres', params || '{"password": "pw29-test-only"}');
    EXCEPTION WHEN insufficient_privilege THEN failed := true;
    END;
    IF NOT failed THEN RAISE EXCEPTION 'refuse: og_create_datasource stored an inline password'; END IF;
    -- … and moving an existing datasource to an inline password.
    failed := false;
    BEGIN
        PERFORM og_fabric('ALTER DATASOURCE inl29 SET CREDENTIALS = {password: ''another-test-pw''}');
    EXCEPTION WHEN insufficient_privilege THEN failed := true;
    END;
    IF NOT failed THEN RAISE EXCEPTION 'refuse: ALTER … SET CREDENTIALS stored an inline password'; END IF;

    -- A reference is fine under refuse, and carries no warning.
    SELECT og_fabric(format('CREATE DATASOURCE ref29 WITH ENGINE = ''postgres'', PARAMETERS = %s',
                            params || '{"credentials": "test:ref29"}')) INTO got;
    IF got->>'result' LIKE '%stored inline%' THEN RAISE EXCEPTION 'refuse: a reference was warned about: %', got; END IF;
    SELECT count(*) INTO n FROM ref29.tradeitems;
    IF n <> 1 THEN RAISE EXCEPTION 'refuse: the reference datasource does not read'; END IF;
    -- So is moving the inline one onto a reference.
    PERFORM og_fabric('ALTER DATASOURCE inl29 SET CREDENTIALS = {credentials: ''test:ref29''}');
    -- An empty password stores no secret and is not refused.
    PERFORM og_create_datasource('nopw29', 'postgres', jsonb_build_object('host', '127.0.0.1', 'port', current_setting('port'),
                                 'database', current_database(), 'user', current_user, 'password', ''));
    RESET ontological.fabric_inline_passwords;
END $$;

-- Another role cannot turn it off: neither by SET once it is defined …
SET ROLE og_fab29_plain;
DO $$
DECLARE failed bool := false;
BEGIN
    BEGIN
        PERFORM set_config('ontological.fabric_inline_passwords', 'allow', false);
    EXCEPTION WHEN insufficient_privilege THEN failed := true;
    END;
    IF NOT failed THEN RAISE EXCEPTION 'a non-superuser changed ontological.fabric_inline_passwords'; END IF;
END $$;
RESET ROLE;

-- … nor with a value set before it was defined, in a session where the
-- database says refuse — a role that may create datasources, not a superuser.
SELECT og_grant('og_fab29_plain', 'read');
GRANT EXECUTE ON FUNCTION og_create_datasource(text, text, jsonb) TO og_fab29_plain;
SELECT current_database() AS db29, current_user AS su29 \gset
ALTER DATABASE :"db29" SET ontological.fabric_inline_passwords = 'refuse';
\c - og_fab29_plain
SET ontological.fabric_inline_passwords = 'allow';
DO $$
DECLARE failed bool := false;
BEGIN
    BEGIN
        PERFORM og_create_datasource('plain29', 'postgres', '{"host": "127.0.0.1", "user": "x", "password": "not-kept"}');
    EXCEPTION WHEN insufficient_privilege THEN
        failed := SQLERRM LIKE '%a password given inline is refused here%';
    END;
    IF NOT failed THEN RAISE EXCEPTION 'a non-superuser''s early SET turned the refusal off'; END IF;
END $$;
\c - :"su29"
ALTER DATABASE :"db29" RESET ontological.fabric_inline_passwords;
REVOKE EXECUTE ON FUNCTION og_create_datasource(text, text, jsonb) FROM og_fab29_plain;
SELECT og_revoke('og_fab29_plain');
DROP ROLE og_fab29_plain;
