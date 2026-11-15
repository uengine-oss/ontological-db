-- Statement time limit: og.statement_timeout.
--
-- A pattern over virtual nodes is a query on a source system, and a cartesian
-- product of two source tables is one forgotten relationship away. Before the
-- limit such a statement ran until the source finished — for the three-way
-- product below, never — and only killing the backend stopped it. Now it is
-- cancelled at its limit, here and at the source, and the session carries on.
--
-- The source is this database again, through a loopback (as in 13). Nothing
-- here is an expected error (run.sh counts them): each cancellation is caught
-- by name and checked, and any error fails the file.
\set ON_ERROR_STOP on

CREATE SCHEMA src17;
CREATE TABLE src17.item (item_id int PRIMARY KEY, name text);
CREATE TABLE src17.line (line_id int PRIMARY KEY, item_id int, qty int);
INSERT INTO src17.item SELECT g, 'item ' || g FROM generate_series(1, 2000) g;
INSERT INTO src17.line SELECT g, 1 + g % 2000, g % 7 FROM generate_series(1, 5000) g;

SELECT og_create_graph('lim');
SELECT og_create_datasource('loop17', 'postgres', jsonb_build_object(
    'host', 'localhost', 'port', current_setting('port'), 'database', current_database(),
    'user', current_user, 'password', ''));
SELECT og_map_virtual('lim', 'Item', 'loop17', 'src17.item', 'item_id');
SELECT og_map_virtual('lim', 'Line', 'loop17', 'src17.line', 'line_id');
SELECT og_create_virtual_rel('lim', 'OF_ITEM', 'Line', 'item_id', 'Item', 'item_id');

-- Unset, the limit is two minutes. The setting reads PostgreSQL's time units;
-- a bare number is milliseconds, 0 is no limit.
SELECT 1 / (og_statement_timeout() = 120000)::int AS default_two_minutes;
SET og.statement_timeout = '2min';
SELECT 1 / (og_statement_timeout() = 120000)::int AS minutes;
SET og.statement_timeout = 1500;
SELECT 1 / (og_statement_timeout() = 1500)::int AS milliseconds;
SET og.statement_timeout = 0;
SELECT 1 / (og_statement_timeout() = 0)::int AS no_limit;

SET og.statement_timeout = '1s';
DO $$
DECLARE
    started timestamptz := clock_timestamp();
    took interval;
BEGIN
    BEGIN
        -- 2000 × 5000 × 5000 rows: the whole aggregate is sent to the source.
        PERFORM og_cypher('lim', 'MATCH (a:Item), (b:Line), (c:Line) RETURN count(*) AS n');
        RAISE EXCEPTION 'the cartesian product finished — it should have been cancelled';
    EXCEPTION WHEN query_canceled THEN
        IF SQLERRM NOT LIKE '%statement timeout%' THEN RAISE; END IF;
    END;
    took := clock_timestamp() - started;
    IF took > interval '15 seconds' THEN
        RAISE EXCEPTION 'a 1 s limit cancelled the statement only after %', took;
    END IF;

    -- Not pushed down as a whole: the product is formed here, row by row.
    started := clock_timestamp();
    BEGIN
        PERFORM og_cypher('lim',
            'MATCH (a:Item), (b:Line) WHERE a.name STARTS WITH ''item'' RETURN a.name, b.qty ORDER BY a.name');
        RAISE EXCEPTION 'the row-returning product finished — it should have been cancelled';
    EXCEPTION WHEN query_canceled THEN
        IF SQLERRM NOT LIKE '%statement timeout%' THEN RAISE; END IF;
    END;
    IF clock_timestamp() - started > interval '15 seconds' THEN
        RAISE EXCEPTION 'the second cancellation took %', clock_timestamp() - started;
    END IF;
END
$$;

-- Nothing is left working at the source: postgres_fdw cancelled the remote half.
SELECT 1 / (count(*) = 0)::int AS source_query_gone
  FROM pg_stat_activity
 WHERE datname = current_database() AND pid <> pg_backend_pid() AND state = 'active';

-- And the session goes on: the next statement has its own limit, and runs.
SELECT 1 / ((og_cypher('lim', 'MATCH (l:Line)-[:OF_ITEM]->(i:Item) RETURN count(*) AS n') ->> 'n')::int = 5000)::int
       AS next_statement_runs;

-- A stricter statement_timeout of PostgreSQL's own is not loosened by a longer
-- og.statement_timeout: the earlier deadline wins.
SET og.statement_timeout = '10min';
SET statement_timeout = '1s';
DO $$
DECLARE started timestamptz := clock_timestamp();
BEGIN
    BEGIN
        PERFORM og_cypher('lim', 'MATCH (a:Item), (b:Line), (c:Line) RETURN count(*) AS n');
        RAISE EXCEPTION 'statement_timeout was loosened by og.statement_timeout';
    EXCEPTION WHEN query_canceled THEN
        IF clock_timestamp() - started > interval '15 seconds' THEN RAISE; END IF;
    END;
END
$$;
RESET statement_timeout;

-- Fabric statements read sources too, and are held to the same limit.
SET og.statement_timeout = '1s';
DO $$
BEGIN
    PERFORM og_fabric('SHOW COLUMN STATS FROM loop17.line');
    BEGIN
        PERFORM og_cypher('lim', 'MATCH (a:Item), (b:Line), (c:Line) RETURN count(*) AS n');
        RAISE EXCEPTION 'not cancelled';
    EXCEPTION WHEN query_canceled THEN NULL;
    END;
END
$$;

-- A value that is not a duration is refused, not silently replaced.
SET og.statement_timeout = 'soon';
DO $$
BEGIN
    PERFORM og_cypher('lim', 'MATCH (i:Item {item_id: 1}) RETURN i.name AS n');
    RAISE EXCEPTION 'og.statement_timeout = soon was accepted';
EXCEPTION WHEN others THEN
    IF SQLERRM NOT LIKE '%og.statement_timeout%' THEN RAISE; END IF;
END
$$;
RESET og.statement_timeout;
SELECT 1 / (og_statement_timeout() = 120000)::int AS back_to_default;
