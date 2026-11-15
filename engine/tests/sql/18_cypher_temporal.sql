-- date(), datetime(), duration() and .year/.month/.day — and on virtual nodes
-- a comparison with a source's date column is sent to the source.
--
-- Before: `date` was an unknown function, `datetime('…')` ignored its argument
-- and returned now(), and `x.year` was "property access is only supported on
-- pattern variables". No expected errors here: any error fails the file.
\set ON_ERROR_STOP on

CREATE SCHEMA src18;
CREATE TABLE src18.orders (order_id int PRIMARY KEY, placed date, due timestamptz, placed_text text);
INSERT INTO src18.orders VALUES
    (1, '2015-12-31', '2016-01-02 10:00+00', '2015-12-31'),
    (2, '2016-01-01', '2016-01-05 10:00+00', '2016-01-01'),
    (3, '2016-03-15', NULL,                  '2016-03-15'),
    (4, '2017-07-04', '2017-07-10 00:00+00', '2017-07-04');

SELECT og_create_graph('cal');
SELECT og_create_datasource('loop18', 'postgres', jsonb_build_object(
    'host', 'localhost', 'port', current_setting('port'), 'database', current_database(),
    'user', current_user, 'password', ''));
SELECT og_map_virtual('cal', 'Order', 'loop18', 'src18.orders', 'order_id');

DO $$
DECLARE got jsonb; n int;
BEGIN
    -- A string becomes a date, and compares with a date column.
    SELECT count(*) INTO n FROM og_cypher('cal',
        'MATCH (o:Order) WHERE o.placed >= date(''2016-01-01'') RETURN o.order_id');
    IF n <> 3 THEN RAISE EXCEPTION 'placed >= date(''2016-01-01''): expected 3, got %', n; END IF;

    -- … and that comparison is part of what the source is sent.
    SELECT count(*) INTO n FROM og_fabric_explain('cal',
        'MATCH (o:Order) WHERE o.placed >= date(''2016-01-01'') RETURN o.order_id')
     WHERE place = 'remote:loop18' AND detail LIKE '%placed >= ''2016-01-01''%';
    IF n <> 1 THEN RAISE EXCEPTION 'the date comparison should reach the source'; END IF;

    -- Fields of a date column, in RETURN and in WHERE.
    SELECT og_cypher('cal', 'MATCH (o:Order {order_id: 3})
        RETURN o.placed.year AS y, o.placed.month AS m, o.placed.day AS d,
               o.placed.quarter AS q, o.placed.dayOfWeek AS w') INTO got;
    IF got <> '{"y": 2016, "m": 3, "d": 15, "q": 1, "w": 2}'::jsonb THEN
        RAISE EXCEPTION 'date fields: %', got;
    END IF;
    SELECT count(*) INTO n FROM og_cypher('cal', 'MATCH (o:Order) WHERE o.placed.year = 2016 RETURN o.order_id');
    IF n <> 2 THEN RAISE EXCEPTION '.year in WHERE: expected 2, got %', n; END IF;

    -- date() is today; datetime() is now.
    SELECT og_cypher('cal', 'RETURN date() AS d, date(datetime()) AS e') INTO got;
    IF (got->>'d')::date <> current_date OR (got->>'e')::date <> current_date THEN
        RAISE EXCEPTION 'date() / datetime(): %', got;
    END IF;

    -- datetime(string) is that instant, not now — compared with a timestamptz column.
    SELECT count(*) INTO n FROM og_cypher('cal',
        'MATCH (o:Order) WHERE o.due > datetime(''2016-01-03T00:00:00Z'') RETURN o.order_id');
    IF n <> 2 THEN RAISE EXCEPTION 'due > datetime(…): expected 2, got %', n; END IF;

    -- A map of fields, through WITH; the value keeps its type across the horizon.
    SELECT og_cypher('cal', 'WITH date({year: 2024, month: 2, day: 29}) AS d
        RETURN d.year AS y, d.dayOfWeek AS w, d.ordinalDay AS od, toString(d) AS s') INTO got;
    IF got <> '{"y": 2024, "w": 4, "od": 60, "s": "2024-02-29"}'::jsonb THEN
        RAISE EXCEPTION 'date({…}) through WITH: %', got;
    END IF;

    -- A date column carried through WITH (it is jsonb there) still has fields.
    SELECT count(*) INTO n FROM og_cypher('cal', 'MATCH (o:Order) WITH o.placed AS p WHERE p.year = 2016 RETURN p');
    IF n <> 2 THEN RAISE EXCEPTION 'field of a WITH value: expected 2, got %', n; END IF;

    -- A map's `year` is still its key, not a date field.
    SELECT og_cypher('cal', 'UNWIND [{year: 7}] AS r RETURN r.year AS y') INTO got;
    IF got <> '{"y": 7}'::jsonb THEN RAISE EXCEPTION 'map key year: %', got; END IF;

    -- A parameter and an unwound string become dates.
    SELECT count(*) INTO n FROM og_cypher('cal',
        'MATCH (o:Order) WHERE o.placed < date($cut) RETURN o.order_id', '{"cut": "2016-01-01"}');
    IF n <> 1 THEN RAISE EXCEPTION 'date($cut): expected 1, got %', n; END IF;
    SELECT og_cypher('cal', 'UNWIND [''2024-03-05''] AS s RETURN date(s).month AS m, s.day AS d') INTO got;
    IF got <> '{"m": 3, "d": 5}'::jsonb THEN RAISE EXCEPTION 'unwound date string: %', got; END IF;

    -- A source that keeps dates as text compares against a date.
    SELECT count(*) INTO n FROM og_cypher('cal',
        'MATCH (o:Order) WHERE o.placed_text < date(''2016-01-01'') RETURN o.order_id');
    IF n <> 1 THEN RAISE EXCEPTION 'text column < date: expected 1, got %', n; END IF;

    -- A date moved by a duration is a date.
    SELECT og_cypher('cal', 'RETURN date(''2024-03-01'') - duration({days: 1}) AS a,
                                   date(''2024-01-31'') + duration(''P1M'') AS b') INTO got;
    IF got <> '{"a": "2024-02-29", "b": "2024-02-29"}'::jsonb THEN
        RAISE EXCEPTION 'date ± duration: %', got;
    END IF;
    SELECT count(*) INTO n FROM og_cypher('cal',
        'MATCH (o:Order) WHERE o.placed >= date(''2016-03-16'') - duration({days: 1}) RETURN o.order_id');
    IF n <> 2 THEN RAISE EXCEPTION 'placed >= date - duration: expected 2, got %', n; END IF;

    -- On the write path too: the stored value is the date, and it has fields.
    PERFORM og_cypher('cal', 'CREATE (:Event {name: ''launch'', day: date(''2024-05-06'')})');
    SELECT og_cypher('cal', 'MATCH (e:Event) RETURN e.day AS d, e.day.month AS m') INTO got;
    IF got <> '{"d": "2024-05-06", "m": 5}'::jsonb THEN RAISE EXCEPTION 'written date: %', got; END IF;
END
$$;

-- A string that is not a date is an error, not a silent null.
DO $$
BEGIN
    PERFORM og_cypher('cal', 'RETURN date(''soon'') AS d');
    RAISE EXCEPTION 'date(''soon'') was accepted';
EXCEPTION WHEN invalid_datetime_format OR datetime_field_overflow THEN NULL;
END
$$;
