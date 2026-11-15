-- Source profiling: the measurements a relationship suggestion is made from.
--
-- SHOW COLUMN STATS and SHOW VALUE OVERLAP read a datasource's tables through
-- the connector (a loopback here, as in 14) and decide nothing. No EXPECT_ERROR:
-- any error fails the file.
\set ON_ERROR_STOP on

CREATE SCHEMA src3;
CREATE TABLE src3.supplier (supplier_id int PRIMARY KEY, name text, status text, code varchar(10));
CREATE TABLE src3.purchase_order (po_id int PRIMARY KEY, supplier_id bigint, status text, priority int, supplier_code text);
INSERT INTO src3.supplier
    SELECT i, 'supplier ' || i, CASE WHEN i % 2 = 0 THEN 'ACTIVE' ELSE 'HOLD' END, 'S' || i FROM generate_series(1, 30) i;
INSERT INTO src3.purchase_order
    SELECT i, 1 + (i % 12), CASE WHEN i % 3 = 0 THEN 'HOLD' ELSE 'OPEN' END, 1 + i % 3, 'S' || (1 + i % 12)
      FROM generate_series(1, 60) i;
UPDATE src3.purchase_order SET supplier_id = NULL WHERE po_id IN (5, 6);

SELECT og_fabric(format(
  'CREATE DATASOURCE prof WITH ENGINE = ''postgres'', PARAMETERS = %s',
  jsonb_build_object('host', 'localhost', 'port', current_setting('port'), 'database', current_database(),
                     'schema', 'src3', 'user', current_user, 'password', '')));

DO $$
DECLARE got jsonb; n int;
BEGIN
    -- one row per column of the table
    SELECT count(*) INTO n FROM og_fabric('SHOW COLUMN STATS FROM prof.purchase_order');
    IF n <> 5 THEN RAISE EXCEPTION 'COLUMN STATS: expected 5 columns, got %', n; END IF;

    SELECT r INTO got FROM og_fabric('show column stats from prof.purchase_order') r WHERE r->>'column' = 'po_id';
    IF NOT (got->>'unique')::boolean OR (got->>'sampled')::int <> 60 OR (got->>'distinct')::int <> 60 THEN
        RAISE EXCEPTION 'po_id should be unique over 60 rows: %', got;
    END IF;
    SELECT r INTO got FROM og_fabric('SHOW COLUMN STATS FROM prof.purchase_order') r WHERE r->>'column' = 'supplier_id';
    IF (got->>'unique')::boolean OR (got->>'nulls')::int <> 2 OR (got->>'distinct')::int <> 12 OR got->>'type' <> 'bigint' THEN
        RAISE EXCEPTION 'supplier_id stats: %', got;
    END IF;
    SELECT r INTO got FROM og_fabric('SHOW COLUMN STATS FROM prof.purchase_order') r WHERE r->>'column' = 'status';
    IF got->'top_values'->0->>0 <> 'OPEN' OR (got->'top_values'->0->>1)::int <> 40 OR (got->>'distinct')::int <> 2 THEN
        RAISE EXCEPTION 'status top values: %', got;
    END IF;
    -- SAMPLE limits the rows read
    SELECT r INTO got FROM og_fabric('SHOW COLUMN STATS FROM prof.purchase_order SAMPLE 10') r WHERE r->>'column' = 'po_id';
    IF (got->>'sampled')::int <> 10 OR jsonb_array_length(got->'samples') <> 10 THEN
        RAISE EXCEPTION 'SAMPLE 10: %', got;
    END IF;

    -- a real reference: every child value is a parent key, and the parent is unique (bigint vs int: typed)
    SELECT r INTO got FROM og_fabric('SHOW VALUE OVERLAP FROM prof.purchase_order(supplier_id) IN prof.supplier(supplier_id)') r;
    IF (got->>'sampled')::int <> 12 OR (got->>'matched')::int <> 12 OR (got->>'ratio')::float8 <> 1
       OR NOT (got->>'parent_unique')::boolean OR got->>'compare' <> 'typed' THEN
        RAISE EXCEPTION 'po.supplier_id -> supplier.supplier_id: %', got;
    END IF;
    -- a text code against a varchar key: compared as text
    SELECT r INTO got FROM og_fabric('SHOW VALUE OVERLAP FROM prof.purchase_order(supplier_code) IN prof.supplier(code) SAMPLE 5') r;
    IF (got->>'sampled')::int <> 5 OR (got->>'ratio')::float8 <> 1 OR got->>'compare' <> 'text' THEN
        RAISE EXCEPTION 'supplier_code -> code: %', got;
    END IF;
    -- the trap: status values overlap completely, but the "parent" is not unique
    SELECT r INTO got FROM og_fabric('SHOW VALUE OVERLAP FROM prof.purchase_order(status) IN prof.supplier(status)') r;
    IF (got->>'parent_unique')::boolean THEN RAISE EXCEPTION 'status should not look like a key: %', got; END IF;
    -- small integer codes against a unique key: the overlap is real, uniqueness alone does not rule it out
    -- (that is what the caller's name and cardinality checks are for)
    SELECT r INTO got FROM og_fabric('SHOW VALUE OVERLAP FROM prof.purchase_order(priority) IN prof.supplier(supplier_id)') r;
    IF (got->>'sampled')::int <> 3 OR (got->>'ratio')::float8 <> 1 THEN RAISE EXCEPTION 'priority overlap: %', got; END IF;

    IF og_cypher_columns('SHOW VALUE OVERLAP FROM a.b(c) IN d.e(f)')
       <> ARRAY['sampled','matched','ratio','parent_sampled','parent_distinct','parent_unique','parent_key','compare'] THEN
        RAISE EXCEPTION 'VALUE OVERLAP columns: %', og_cypher_columns('SHOW VALUE OVERLAP FROM a.b(c) IN d.e(f)');
    END IF;

    -- refused: an unknown table or column, a sample that is not a number
    BEGIN
        PERFORM og_fabric('SHOW COLUMN STATS FROM prof.nothing');
        RAISE EXCEPTION 'unknown table accepted';
    EXCEPTION WHEN OTHERS THEN
        IF SQLERRM NOT LIKE '%no table ''nothing''%' THEN RAISE; END IF;
    END;
    BEGIN
        PERFORM og_fabric('SHOW VALUE OVERLAP FROM prof.purchase_order(nope) IN prof.supplier(supplier_id)');
        RAISE EXCEPTION 'unknown column accepted';
    EXCEPTION WHEN OTHERS THEN
        IF SQLERRM NOT LIKE '%no column ''nope''%' THEN RAISE; END IF;
    END;
    BEGIN
        PERFORM og_fabric('SHOW COLUMN STATS FROM prof.supplier SAMPLE many');
        RAISE EXCEPTION 'SAMPLE many accepted';
    EXCEPTION WHEN OTHERS THEN
        IF SQLERRM NOT LIKE '%SAMPLE takes a number%' THEN RAISE; END IF;
    END;
END $$;

-- Profiling is a read: nothing was written at the source.
SELECT 1 / (SELECT (count(*) = 60)::int FROM src3.purchase_order) AS source_untouched;
SELECT og_fabric('DROP DATASOURCE prof');
