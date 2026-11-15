-- toInteger() / toFloat() / toString() as Cypher defines them, so sum(toInteger(x)) works.
--
-- Both compiled to a bare SQL cast: `::int8` rounds (toInteger(2.5) was 3, Cypher
-- says 2), fails the whole query on one string that is not a number, and cannot
-- read a jsonb value at all — "cannot cast jsonb string to type bigint" for any
-- x that came from a map, an UNWIND, or a WITH over a property. Now a number is
-- truncated toward zero, a numeric string is read, anything else is null.
-- No expected errors here: any error fails the file.
\set ON_ERROR_STOP on

CREATE SCHEMA src20;
CREATE TABLE src20.line (line_id int PRIMARY KEY, qty_text text, price numeric, weight float8);
INSERT INTO src20.line VALUES (1, '3', 2.5, 1.9), (2, ' 4 ', 2.9, -1.9), (3, 'n/a', -2.5, 0.5), (4, '5.8', 10, 2), (5, NULL, NULL, NULL);

SELECT og_create_graph('num');
SELECT og_create_datasource('loop20', 'postgres', jsonb_build_object(
    'host', 'localhost', 'port', current_setting('port'), 'database', current_database(),
    'user', current_user, 'password', ''));
SELECT og_map_virtual('num', 'Line', 'loop20', 'src20.line', 'line_id');

DO $$
DECLARE got jsonb;
BEGIN
    -- A text column: numbers are read (5.8 → 5), 'n/a' is null.
    SELECT og_cypher('num', 'MATCH (l:Line) RETURN sum(toInteger(l.qty_text)) AS s') INTO got;
    IF (got->>'s')::int <> 12 THEN RAISE EXCEPTION 'sum over a text column: %', got; END IF;

    -- Numbers truncate toward zero: 2.5 → 2, 2.9 → 2, -2.5 → -2.
    SELECT og_cypher('num', 'MATCH (l:Line) RETURN sum(toInteger(l.price)) AS s, sum(toInteger(l.weight)) AS w') INTO got;
    IF got <> '{"s": 12, "w": 2}'::jsonb THEN RAISE EXCEPTION 'truncation: %', got; END IF;

    -- jsonb from a WITH over a property, and from UNWIND.
    SELECT og_cypher('num', 'MATCH (l:Line) WITH l.qty_text AS q RETURN sum(toInteger(q)) AS s') INTO got;
    IF (got->>'s')::int <> 12 THEN RAISE EXCEPTION 'sum through WITH: %', got; END IF;
    SELECT og_cypher('num', 'UNWIND [''1'', ''2'', ''3'', ''x'', null, 2.9, -2.9, true] AS x
                             RETURN sum(toInteger(x)) AS s, collect(toInteger(x)) AS xs') INTO got;
    IF got <> '{"s": 7, "xs": [1, 2, 3, 2, -2, 1]}'::jsonb THEN RAISE EXCEPTION 'over UNWIND: %', got; END IF;

    -- A parameter arrives as text either way.
    SELECT og_cypher('num', 'RETURN toInteger($a) AS a, toInteger($b) AS b', '{"a": 41.9, "b": "7"}') INTO got;
    IF got <> '{"a": 41, "b": 7}'::jsonb THEN RAISE EXCEPTION 'parameters: %', got; END IF;

    -- toFloat follows the same rules.
    SELECT og_cypher('num', 'UNWIND [''1.5'', ''1e3'', ''x'', 2] AS x RETURN sum(toFloat(x)) AS s, toFloat(''abc'') AS n') INTO got;
    IF got <> '{"s": 1003.5, "n": null}'::jsonb THEN RAISE EXCEPTION 'toFloat: %', got; END IF;

    -- toString of a carried value is the string, not its JSON spelling.
    SELECT og_cypher('num', 'UNWIND [''abc'', 12, 2.5, true] AS x RETURN collect(toString(x)) AS xs') INTO got;
    IF got <> '{"xs": ["abc", "12", "2.5", "true"]}'::jsonb THEN RAISE EXCEPTION 'toString over UNWIND: %', got; END IF;
    SELECT og_cypher('num', 'MATCH (l:Line {line_id: 2}) WITH l.qty_text AS q RETURN toString(q) AS s') INTO got;
    IF got <> '{"s": " 4 "}'::jsonb THEN RAISE EXCEPTION 'toString through WITH: %', got; END IF;

    -- An integer column is still a plain cast — and a filter on it reaches the source.
    SELECT og_cypher('num', 'MATCH (l:Line) WHERE toInteger(l.line_id) <= 2 RETURN count(*) AS n') INTO got;
    IF (got->>'n')::int <> 2 THEN RAISE EXCEPTION 'integer column: %', got; END IF;
END
$$;
