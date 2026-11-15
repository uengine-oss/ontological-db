-- What the source says about its tables, which the import does not keep:
-- SHOW TABLES says table / view / materialized / foreign, SHOW TABLES, SHOW
-- COLUMNS and SHOW COLUMN STATS say the primary key at the source (separate
-- from `key`, the column a virtual node uses as its id). A datasource whose
-- source catalog is unknown answers null. An inline password gets a warning.
-- The source is this database again, through a postgres_fdw loopback.
\set ON_ERROR_STOP on

CREATE SCHEMA srcdb;
CREATE TABLE srcdb.item (id int PRIMARY KEY, name text, shelf text);
CREATE TABLE srcdb.line (order_no text, line_no int, item_id int, PRIMARY KEY (order_no, line_no));
CREATE TABLE srcdb.loose (a int, b text);
CREATE VIEW srcdb.item_names AS SELECT id, name FROM srcdb.item;
CREATE MATERIALIZED VIEW srcdb.item_count AS SELECT count(*) AS n FROM srcdb.item;
INSERT INTO srcdb.item VALUES (1, 'a', 'x'), (2, 'b', 'y');

SELECT og_fabric('CREATE GRAPH fabx');

DO $$
DECLARE got jsonb; msg text; n int;
BEGIN
    SELECT og_fabric(format(
      'CREATE DATASOURCE srcx WITH ENGINE = ''postgres'', PARAMETERS = %s',
      jsonb_build_object('host', 'localhost', 'port', current_setting('port'), 'database', current_database(),
                         'schema', 'srcdb', 'user', current_user, 'password', ''))) INTO got;
    -- An empty password is no secret: no warning.
    IF got->>'result' LIKE '%WARNING%' THEN RAISE EXCEPTION 'an empty password was warned about: %', got; END IF;

    -- (a materialized view has no foreign table of its own to import; postgres_fdw
    -- imports relations of kind r, v, m, f, p alike)
    SELECT jsonb_object_agg(r->>'table', r->'kind') INTO got FROM og_fabric('SHOW TABLES FROM srcx') r;
    IF got->'srcx.item' <> '"table"' OR got->'srcx.item_names' <> '"view"' OR got->'srcx.item_count' <> '"materialized"' THEN
        RAISE EXCEPTION 'SHOW TABLES kind: %', got;
    END IF;
    SELECT jsonb_object_agg(r->>'table', r->'primary_key') INTO got FROM og_fabric('SHOW TABLES FROM srcx') r;
    IF got->'srcx.item' <> '["id"]' OR got->'srcx.line' <> '["order_no", "line_no"]'
       OR got->'srcx.loose' <> '[]' OR got->'srcx.item_names' <> '[]' THEN
        RAISE EXCEPTION 'SHOW TABLES primary_key: %', got;
    END IF;
    IF og_cypher_columns('SHOW TABLES FROM srcx') <> ARRAY['table','columns','virtual_node','kind','primary_key'] THEN
        RAISE EXCEPTION 'SHOW TABLES columns: %', og_cypher_columns('SHOW TABLES FROM srcx');
    END IF;

    -- A virtual node keyed on another column: `key` and `primary_key` differ.
    PERFORM og_fabric('CREATE VIRTUAL NODE Item FROM srcx.item KEY name');
    SELECT jsonb_object_agg(r->>'column', jsonb_build_array(r->'key', r->'primary_key')) INTO got FROM og_fabric('SHOW COLUMNS FROM srcx.item') r;
    IF got <> '{"id": [false, true], "name": [true, false], "shelf": [false, false]}' THEN
        RAISE EXCEPTION 'SHOW COLUMNS key vs primary_key: %', got;
    END IF;
    SELECT jsonb_object_agg(r->>'column', r->'primary_key') INTO got FROM og_fabric('SHOW COLUMNS FROM srcx.line') r;
    IF got <> '{"order_no": true, "line_no": true, "item_id": false}' THEN RAISE EXCEPTION 'composite key: %', got; END IF;
    SELECT jsonb_object_agg(r->>'column', jsonb_build_array(r->'key', r->'primary_key')) INTO got FROM og_fabric('SHOW COLUMN STATS FROM srcx.item') r;
    IF got <> '{"id": [false, true], "name": [true, false], "shelf": [false, false]}' THEN
        RAISE EXCEPTION 'SHOW COLUMN STATS primary_key: %', got;
    END IF;
    IF og_cypher_columns('SHOW COLUMNS FROM srcx.item') <> ARRAY['column','type','nullable','key','primary_key'] THEN
        RAISE EXCEPTION 'SHOW COLUMNS columns';
    END IF;

    -- A table added at the source later is known after REFRESH.
    CREATE TABLE srcdb.later (z int PRIMARY KEY);
    PERFORM og_fabric('REFRESH DATASOURCE srcx');
    SELECT og_fabric('SHOW COLUMNS FROM srcx.later') INTO got;
    IF got->'primary_key' <> 'true' THEN RAISE EXCEPTION 'primary key after REFRESH: %', got; END IF;

    -- Unknown is null, not false: a datasource without the source catalog.
    UPDATE og_catalog.datasource SET options = options - 'source_catalog' WHERE name = 'srcx';
    SELECT jsonb_agg(DISTINCT coalesce(r->>'kind', 'null') || '/' || coalesce(r->>'primary_key', 'null')) INTO got FROM og_fabric('SHOW TABLES FROM srcx') r;
    IF got <> '["null/null"]' THEN RAISE EXCEPTION 'unknown kind / primary key should be null: %', got; END IF;
    SELECT jsonb_agg(DISTINCT r->'primary_key') INTO got FROM og_fabric('SHOW COLUMNS FROM srcx.item') r;
    IF got <> '[null]' THEN RAISE EXCEPTION 'unknown column primary_key should be null: %', got; END IF;

    -- An inline password warns in the result (and never reaches the catalog).
    SELECT og_fabric(format(
      'CREATE DATASOURCE srcw WITH ENGINE = ''postgres'', PARAMETERS = %s',
      jsonb_build_object('host', 'localhost', 'port', current_setting('port'), 'database', current_database(),
                         'schema', 'srcdb', 'user', current_user, 'password', 'not-a-real-secret'))) INTO got;
    IF got->>'result' NOT LIKE '% WARNING: the password is stored inline%credentials: ''<reference>''%' THEN
        RAISE EXCEPTION 'inline password without a warning: %', got;
    END IF;
    IF got::text LIKE '%not-a-real-secret%' THEN RAISE EXCEPTION 'the warning leaked the password'; END IF;
    -- A reference is not warned about (its own test is 15); a refused statement leaves no probe schema behind.
    SELECT count(*) INTO n FROM pg_namespace WHERE nspname = 'og_probe';
    IF n <> 0 THEN RAISE EXCEPTION 'the catalog probe left its schema behind'; END IF;
END $$;
