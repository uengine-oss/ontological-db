-- Fabric statements: the SQL dialect the gateway relays, translated here.
--
-- `og_fabric(text)` translates and runs one statement, so the grammar is
-- tested without a gateway in front of it. The source is this database again,
-- through a postgres_fdw loopback, as in 07.
\set ON_ERROR_STOP on

CREATE SCHEMA src;
CREATE TABLE src.supplier (code text PRIMARY KEY, name text, country text);
CREATE TABLE src.purchase (po_no text PRIMARY KEY, supplier_code text, amount numeric);
INSERT INTO src.supplier VALUES ('S1','Acme','KR'), ('S2','Bolt','CN');
INSERT INTO src.purchase VALUES ('P1','S1',100), ('P2','S1',300), ('P3','S2',50);

SELECT og_fabric('CREATE GRAPH fab');

-- The parameters are JSON; build them, since port and database vary by run.
SELECT og_fabric(format(
  'CREATE DATASOURCE loop WITH ENGINE = ''postgres'', PARAMETERS = %s',
  jsonb_build_object('host', 'localhost', 'port', current_setting('port'), 'database', current_database(),
                     'schema', 'src', 'user', current_user, 'password', '')));

DO $$
DECLARE n int; got jsonb; sql text;
BEGIN
    -- The datasource's tables are local relations under its own name.
    SELECT count(*) INTO n FROM loop.supplier;
    IF n <> 2 THEN RAISE EXCEPTION 'loop.supplier should read the source: got % rows', n; END IF;
    SELECT count(*) INTO n FROM og_fabric('SHOW TABLES FROM loop');
    IF n <> 2 THEN RAISE EXCEPTION 'SHOW TABLES: expected 2, got %', n; END IF;

    PERFORM og_fabric('CREATE VIRTUAL NODE Supplier FROM loop.supplier KEY code');
    PERFORM og_fabric('create virtual node Purchase from loop.purchase key po_no properties (po_no, supplier_code, amount AS total);');
    PERFORM og_fabric('CREATE VIRTUAL RELATIONSHIP FROM_SUPPLIER FROM Purchase(supplier_code) TO Supplier(code)');

    SELECT og_fabric('SHOW VIRTUAL RELATIONSHIPS') INTO got;
    IF got->>'from' <> 'Purchase.supplier_code' OR got->>'to' <> 'Supplier.code' THEN
        RAISE EXCEPTION 'SHOW VIRTUAL RELATIONSHIPS: %', got;
    END IF;

    -- A Cypher query is a statement too; PROPERTIES renamed a column.
    SELECT og_fabric('MATCH (p:Purchase)-[:FROM_SUPPLIER]->(s:Supplier {code: ''S1''}) RETURN s.name AS supplier, sum(p.total) AS total') INTO got;
    IF got->>'supplier' <> 'Acme' OR (got->>'total')::numeric <> 400 THEN
        RAISE EXCEPTION 'Cypher through og_fabric: %', got;
    END IF;

    -- The translation is plain SQL with one column per RETURN item, in order.
    sql := og_fabric_translate('MATCH (s:Supplier) RETURN s.name, count(*) AS n');
    IF sql NOT LIKE 'SELECT r ->> ''s.name'' AS "s.name", r ->> ''n'' AS "n" FROM og_cypher(%' THEN
        RAISE EXCEPTION 'translation shape: %', sql;
    END IF;

    -- Leading comments and a trailing semicolon are not part of the statement.
    SELECT count(*) INTO n FROM og_fabric(E'-- which sources?\n  SHOW DATASOURCES ;');
    IF n <> 1 THEN RAISE EXCEPTION 'SHOW DATASOURCES: expected 1, got %', n; END IF;

    SELECT count(*) INTO n FROM og_fabric('EXPLAIN FABRIC MATCH (p:Purchase)-[:FROM_SUPPLIER]->(s:Supplier) RETURN s.name')
     WHERE og_fabric->>'place' = 'remote:loop';
    IF n < 1 THEN RAISE EXCEPTION 'EXPLAIN FABRIC should report a remote step'; END IF;

    -- The same statements arrive wherever Cypher does — the Studio and Bolt
    -- both call og_cypher — with the caller's graph and its parameters.
    SELECT count(*) INTO n FROM og_cypher('fab', 'SHOW VIRTUAL NODES');
    IF n <> 2 THEN RAISE EXCEPTION 'SHOW VIRTUAL NODES through og_cypher: expected 2, got %', n; END IF;
    IF og_cypher_columns('SHOW DATASOURCES') <> ARRAY['name','engine','host','tables'] THEN
        RAISE EXCEPTION 'column order of a fabric statement: %', og_cypher_columns('SHOW DATASOURCES');
    END IF;
    IF (og_cypher_check('SHOW DATASOURCES')->>'write')::bool OR NOT (og_cypher_check('DROP DATASOURCE x')->>'write')::bool THEN
        RAISE EXCEPTION 'read/write verdict of fabric statements is wrong';
    END IF;

    -- Settings as a Cypher map, the password as a parameter. The secret must
    -- reach the user mapping and nothing a reader can see.
    PERFORM og_cypher('fab', format(
        'CREATE DATASOURCE loop2 WITH ENGINE = ''postgres'', PARAMETERS = {host: ''localhost'', port: %s, database: %L, schema: ''src'', user: %L, password: $pw}',
        current_setting('port'), current_database(), current_user::text), '{"pw": "sekret-xyz"}');
    SELECT count(*) INTO n FROM loop2.purchase;
    IF n <> 3 THEN RAISE EXCEPTION 'datasource from a Cypher map: expected 3 rows, got %', n; END IF;
    IF EXISTS (SELECT 1 FROM og_data.og_audit WHERE query LIKE '%sekret%')
       OR EXISTS (SELECT 1 FROM og_catalog.datasource WHERE options::text LIKE '%sekret%') THEN
        RAISE EXCEPTION 'a datasource password leaked into the audit trail or the catalog';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM og_data.og_audit WHERE lang = 'fabric' AND query LIKE '%CREATE DATASOURCE loop2%{…}%') THEN
        RAISE EXCEPTION 'a fabric statement should be audited with its parameter block blanked';
    END IF;
    IF og_cypher_sql('fab', 'CREATE DATASOURCE z WITH ENGINE = ''mysql'', PARAMETERS = {host: ''h'', user: ''u'', password: ''p''}') LIKE '%''p''%' THEN
        RAISE EXCEPTION 'the shown SQL of CREATE DATASOURCE must not carry its parameters';
    END IF;
    PERFORM og_cypher('fab', 'DROP DATASOURCE loop2');

    -- A node defined by a query: the join and the aggregate, as a label.
    PERFORM og_cypher('fab', 'CREATE VIRTUAL NODE Spend KEY code AS
        SELECT s.code, sum(p.amount) AS total FROM loop.supplier s JOIN loop.purchase p ON p.supplier_code = s.code GROUP BY s.code');
    PERFORM og_cypher('fab', 'CREATE VIRTUAL RELATIONSHIP SPEND_OF FROM Spend(code) TO Supplier(code)');
    SELECT og_cypher('fab', 'MATCH (x:Spend)-[:SPEND_OF]->(s:Supplier {code: ''S1''}) RETURN x.total AS total') INTO got;
    IF (got->>'total')::numeric <> 400 THEN RAISE EXCEPTION 'query-defined node: expected 400, got %', got; END IF;
    PERFORM og_cypher('fab', 'DROP VIRTUAL RELATIONSHIP SPEND_OF');
    PERFORM og_cypher('fab', 'DROP VIRTUAL NODE Spend');
    IF to_regclass('og_views.fab__spend') IS NOT NULL THEN RAISE EXCEPTION 'the view of a dropped query-defined node remains'; END IF;

    -- Materialized, a node type is listed as such and can be written.
    PERFORM og_fabric('MATERIALIZE VIRTUAL NODE Supplier');
    SELECT og_fabric('SHOW VIRTUAL NODES') INTO got;  -- first row: Purchase, still virtual
    SELECT count(*) INTO n FROM og_fabric('SHOW VIRTUAL NODES') r WHERE r->>'storage' = 'materialized';
    IF n <> 1 THEN RAISE EXCEPTION 'one node type should be materialized, got %', n; END IF;
    PERFORM og_fabric('MATCH (s:Supplier {code: ''S2''}) SET s.name = ''Bolt Ltd''');
END
$$;

-- What must be refused, each reached in turn.
\set ON_ERROR_STOP off

-- EXPECT_ERROR: a datasource that virtual types still read cannot be dropped.
SELECT og_fabric('DROP DATASOURCE loop');
-- EXPECT_ERROR: nor a node type that a relationship still joins on.
SELECT og_fabric('DROP VIRTUAL NODE Purchase');
-- EXPECT_ERROR: a statement that stops short says what it expected.
SELECT og_fabric('CREATE VIRTUAL NODE X FROM loop.supplier');
-- EXPECT_ERROR: ordinary SQL is not the translator's to run.
SELECT og_fabric('SELECT 1');
-- EXPECT_ERROR: the query of a node is one SELECT; nothing may ride behind it.
SELECT og_cypher('fab', 'CREATE VIRTUAL NODE Sneaky KEY code AS SELECT 1 AS code; DROP TABLE src.purchase');
-- The fabric does not write to sources. The loopback account is a superuser
-- that *could*, which is the case this has to hold in — for plain SQL too.
-- EXPECT_ERROR: DELETE through a datasource table.
DELETE FROM loop.purchase;
-- EXPECT_ERROR: UPDATE.
UPDATE loop.purchase SET amount = 0;
-- EXPECT_ERROR: INSERT.
INSERT INTO loop.purchase VALUES ('P9', 'S1', 1);
-- EXPECT_ERROR: TRUNCATE.
TRUNCATE loop.purchase;
SELECT 1 / (SELECT (count(*) = 3 AND sum(amount) = 450)::int FROM src.purchase) AS source_untouched;

-- EXPECT_ERROR: a datasource may not take a system schema's name.
SELECT og_fabric('CREATE DATASOURCE public WITH ENGINE = ''mysql'', PARAMETERS = {"host":"h","user":"u"}');

-- And in the right order it all comes apart cleanly.
SELECT og_fabric('DROP VIRTUAL RELATIONSHIP FROM_SUPPLIER');
SELECT og_fabric('DROP VIRTUAL NODE Purchase');
SELECT og_fabric('DROP DATASOURCE loop');
SELECT 1 / (SELECT (count(*) = 0)::int FROM og_fabric('SHOW DATASOURCES')) AS no_datasource_left;
SELECT 1 / (to_regnamespace('loop') IS NULL)::int AS schema_gone;
