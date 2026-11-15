-- Credentials of a datasource: a reference instead of the password.
--
-- With `credentials: '<ref>'` the connector reads the password from a file
-- under ontological.fabric_credentials_dir, written by whoever keeps the secret
-- (the desktop app from the OS keychain, a Vault agent on a server). The
-- catalog, the user mapping and the audit trail never hold it.
--
-- The source is this database again, through a loopback, as a role that must
-- log in with a password when the test server's pg_hba asks one of
-- og_fabric_pw over TCP (the desktop's runtime test sets it up so). psql writes
-- the credential files itself (\!), so the server must share this machine's
-- file system, and libpq wants them readable by their owner only.
--
-- No EXPECT_ERROR here: any error fails the file.
\set ON_ERROR_STOP on

CREATE SCHEMA src2;
CREATE TABLE src2.item (id int PRIMARY KEY, name text);
INSERT INTO src2.item VALUES (1, 'bolt'), (2, 'nut'), (3, 'gear');

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'og_fabric_pw') THEN
        CREATE ROLE og_fabric_pw LOGIN;
    END IF;
END $$;
ALTER ROLE og_fabric_pw PASSWORD 's3cret-pw:1';
GRANT USAGE ON SCHEMA src2 TO og_fabric_pw;
GRANT SELECT ON ALL TABLES IN SCHEMA src2 TO og_fabric_pw;

SELECT og_fabric('CREATE GRAPH cred');

-- The keeper writes the file first, here into the data directory. A ':' in a
-- password is escaped in a passfile.
SELECT set_config('ontological.fabric_credentials_dir', current_setting('data_directory'), false);
SELECT current_setting('data_directory') AS datadir \gset
\setenv OG_CRED_DIR :datadir
\! umask 077 && printf '%s\n' '*:*:*:og_fabric_pw:s3cret-pw\:1' > "$OG_CRED_DIR/og_ds_sec.pgpass" && chmod 600 "$OG_CRED_DIR/og_ds_sec.pgpass"

SELECT og_fabric(format(
  'CREATE DATASOURCE sec WITH ENGINE = ''postgres'', PARAMETERS = %s',
  jsonb_build_object('host', '127.0.0.1', 'port', current_setting('port'), 'database', current_database(),
                     'schema', 'src2', 'user', 'og_fabric_pw', 'credentials', 'test:sec',
                     'api_token', 'never-kept')));

DO $$
DECLARE n int; opts text; got jsonb;
BEGIN
    SELECT count(*) INTO n FROM sec.item;
    IF n <> 3 THEN RAISE EXCEPTION 'sec.item through the passfile: expected 3 rows, got %', n; END IF;

    -- the user mapping names the user and nothing secret
    SELECT array_to_string(m.umoptions, ',') INTO opts
      FROM pg_user_mapping m JOIN pg_foreign_server s ON s.oid = m.umserver WHERE s.srvname = 'og_ds_sec';
    IF opts LIKE '%password=%' OR opts NOT LIKE '%user=og_fabric_pw%' THEN
        RAISE EXCEPTION 'user mapping should hold no password: %', opts;
    END IF;
    SELECT array_to_string(srvoptions, ',') INTO opts FROM pg_foreign_server WHERE srvname = 'og_ds_sec';
    IF opts NOT LIKE '%passfile=%og_ds_sec.pgpass%' THEN RAISE EXCEPTION 'server should name its passfile: %', opts; END IF;

    -- the catalog keeps the reference and the file, no secret of any name
    SELECT options INTO got FROM og_catalog.datasource WHERE name = 'sec';
    IF got->>'credentials' <> 'test:sec' OR got->>'credentials_file' NOT LIKE '%/og_ds_sec.pgpass'
       OR got ? 'api_token' OR got ? 'password' THEN
        RAISE EXCEPTION 'catalog options: %', got;
    END IF;

    SELECT r INTO got FROM og_fabric('SHOW DATASOURCES') r WHERE r->>'name' = 'sec';
    IF got->>'credentials' <> 'test:sec' THEN RAISE EXCEPTION 'SHOW DATASOURCES credentials: %', got; END IF;
    SELECT r INTO got FROM og_fabric('show credentials;') r WHERE r->>'name' = 'sec';
    IF got->>'user' <> 'og_fabric_pw' OR got->>'file' NOT LIKE '%/og_ds_sec.pgpass' OR got->>'port' IS NULL THEN
        RAISE EXCEPTION 'SHOW CREDENTIALS: %', got;
    END IF;
    IF og_cypher_columns('SHOW CREDENTIALS') <> ARRAY['name','engine','credentials','user','host','port','database','file'] THEN
        RAISE EXCEPTION 'SHOW CREDENTIALS columns: %', og_cypher_columns('SHOW CREDENTIALS');
    END IF;
END $$;

-- A virtual node over the source, then the source's password changes. Only the
-- file and the credentials change; the node is not dropped.
SELECT og_fabric('CREATE VIRTUAL NODE SecItem IN cred FROM sec.item KEY id');
ALTER ROLE og_fabric_pw PASSWORD 'n3w-pw';
\! umask 077 && printf '%s\n' '*:*:*:og_fabric_pw:n3w-pw' > "$OG_CRED_DIR/og_ds_sec.pgpass" && chmod 600 "$OG_CRED_DIR/og_ds_sec.pgpass"
SELECT og_fabric('ALTER DATASOURCE sec SET CREDENTIALS = {credentials: ''test:sec#2''}');
DO $$
DECLARE n int; got jsonb;
BEGIN
    SELECT (r->>'n')::int INTO n FROM og_cypher('cred', 'MATCH (i:SecItem) RETURN count(i) AS n') r;
    IF n <> 3 THEN RAISE EXCEPTION 'virtual node after the credentials changed: expected 3, got %', n; END IF;
    SELECT options INTO got FROM og_catalog.datasource WHERE name = 'sec';
    IF got->>'credentials' <> 'test:sec#2' THEN RAISE EXCEPTION 'reference not updated: %', got; END IF;
END $$;

-- Back to an inline password, given as a query parameter: it reaches the user
-- mapping (that is what inline means) but not the catalog or the audit trail.
SELECT og_fabric('ALTER DATASOURCE sec SET CREDENTIALS = {user: ''og_fabric_pw'', password: $pw}', '{"pw": "n3w-pw"}');
DO $$
DECLARE n int; opts text; got jsonb; leaked int;
BEGIN
    SELECT (r->>'n')::int INTO n FROM og_cypher('cred', 'MATCH (i:SecItem) RETURN count(i) AS n') r;
    IF n <> 3 THEN RAISE EXCEPTION 'inline password: expected 3, got %', n; END IF;
    SELECT array_to_string(srvoptions, ',') INTO opts FROM pg_foreign_server WHERE srvname = 'og_ds_sec';
    IF opts LIKE '%passfile=%' THEN RAISE EXCEPTION 'passfile should be gone: %', opts; END IF;
    SELECT options INTO got FROM og_catalog.datasource WHERE name = 'sec';
    IF got->>'credentials' <> 'inline' OR got ? 'credentials_file' OR got ? 'password' THEN
        RAISE EXCEPTION 'catalog after inline: %', got;
    END IF;
    SELECT count(*) INTO leaked FROM og_data.og_audit WHERE query LIKE '%n3w-pw%' OR query LIKE '%s3cret%';
    IF leaked <> 0 THEN RAISE EXCEPTION 'a password reached the audit trail (% rows)', leaked; END IF;
END $$;

-- The SQL a CREATE DATASOURCE stands for carries its parameters as $1, not as text.
SELECT 1 / (og_fabric_translate('CREATE DATASOURCE x WITH ENGINE = ''postgres'', PARAMETERS = {host: ''h'', user: ''u'', password: ''p4ss''}') NOT LIKE '%p4ss%')::int
  AS parameters_not_in_sql;

SELECT og_fabric('DROP VIRTUAL NODE SecItem IN cred');
SELECT og_fabric('DROP DATASOURCE sec');
\! rm -f "$OG_CRED_DIR/og_ds_sec.pgpass"

-- Refused: a reference with nowhere to write the file; a password and a reference both.
DO $$
BEGIN
    PERFORM set_config('ontological.fabric_credentials_dir', '', false);
    BEGIN
        PERFORM og_fabric(format(
          'CREATE DATASOURCE nodir WITH ENGINE = ''postgres'', PARAMETERS = %s',
          jsonb_build_object('host', '127.0.0.1', 'database', current_database(), 'schema', 'src2',
                             'user', 'og_fabric_pw', 'credentials', 'test:x')));
        RAISE EXCEPTION 'a reference with no credentials directory was accepted';
    EXCEPTION WHEN OTHERS THEN
        IF SQLERRM NOT LIKE '%fabric_credentials_dir%' THEN RAISE; END IF;
    END;
    BEGIN
        PERFORM og_fabric('CREATE DATASOURCE both WITH ENGINE = ''postgres'', PARAMETERS = {host: ''h'', database: ''d'', user: ''u'', password: ''p'', credentials: ''r''}');
        RAISE EXCEPTION 'a password and a reference together were accepted';
    EXCEPTION WHEN OTHERS THEN
        IF SQLERRM NOT LIKE '%either a password or a credentials reference%' THEN RAISE; END IF;
    END;
END $$;
