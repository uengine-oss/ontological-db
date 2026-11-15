-- CALL { … } subqueries, and functions Neo4j has that the engine did not:
-- range(), left(), right(), ltrim(), rtrim(), reverse(), sign(), nullIf(),
-- isEmpty(), toBoolean() — and point(), refused by name.
--
-- A subquery runs once per incoming row. Reading ones compile to a LATERAL
-- join; writing ones run their body row by row, each run seeing the runs before
-- it. Imports by a leading `WITH a, b` or the scope clause `CALL (a, b)`; only
-- imported variables are visible inside. No rows from the body drops the row
-- (OPTIONAL CALL keeps it with nulls); an aggregate returns a row over nothing;
-- a body without RETURN leaves the rows as they were; UNION inside combines
-- branches. IN TRANSACTIONS is refused.
\set ON_ERROR_STOP on

SELECT og_create_graph('sub');

DO $$
DECLARE got jsonb; failed bool;
BEGIN
    PERFORM og_cypher('sub', $q$CREATE (a:Shelf {name: 'a'})-[:HOLDS]->(:Box {v: 1}), (a)-[:HOLDS]->(:Box {v: 2}),
                                      (:Shelf {name: 'b'}), (c:Shelf {name: 'c'})-[:HOLDS]->(:Box {v: 5})$q$);

    ------------------------------------------------------------- reading subqueries
    SELECT jsonb_agg(r ORDER BY r->>'s') FROM og_cypher('sub', $q$MATCH (s:Shelf)
        CALL { WITH s MATCH (s)-[:HOLDS]->(b) RETURN count(b) AS n, sum(b.v) AS total }
        RETURN s.name AS s, n, total$q$) r INTO got;
    IF got <> '[{"s": "a", "n": 2, "total": 3}, {"s": "b", "n": 0, "total": null}, {"s": "c", "n": 1, "total": 5}]'
        THEN RAISE EXCEPTION 'importing WITH, an aggregate over nothing: %', got; END IF;
    SELECT jsonb_agg(r ORDER BY r->>'s', r->>'v') FROM og_cypher('sub', $q$MATCH (s:Shelf)
        CALL (s) { MATCH (s)-[:HOLDS]->(b) RETURN b.v AS v } RETURN s.name AS s, v$q$) r INTO got;
    IF got <> '[{"s": "a", "v": 1}, {"s": "a", "v": 2}, {"s": "c", "v": 5}]'
        THEN RAISE EXCEPTION 'scope clause: rows multiply, an empty result drops the row: %', got; END IF;
    SELECT jsonb_agg(r ORDER BY r->>'s', r->>'v') FROM og_cypher('sub', $q$MATCH (s:Shelf)
        OPTIONAL CALL (s) { MATCH (s)-[:HOLDS]->(b) RETURN b.v AS v } RETURN s.name AS s, v$q$) r INTO got;
    IF got <> '[{"s": "a", "v": 1}, {"s": "a", "v": 2}, {"s": "b", "v": null}, {"s": "c", "v": 5}]'
        THEN RAISE EXCEPTION 'OPTIONAL CALL: %', got; END IF;
    SELECT jsonb_agg(r ORDER BY r->>'s') FROM og_cypher('sub', $q$MATCH (s:Shelf)
        CALL { WITH s MATCH (s)-[:HOLDS]->(b) RETURN b.v AS top ORDER BY b.v DESC LIMIT 1 } RETURN s.name AS s, top$q$) r INTO got;
    IF got <> '[{"s": "a", "top": 2}, {"s": "c", "top": 5}]' THEN RAISE EXCEPTION 'ORDER BY … LIMIT per row: %', got; END IF;
    SELECT og_cypher('sub', $q$MATCH (s:Shelf {name: 'a'})
        CALL { WITH s MATCH (s)-[:HOLDS]->(b) RETURN b.v AS x UNION WITH s RETURN 100 AS x UNION RETURN 100 AS x }
        RETURN collect(x) AS xs$q$) INTO got;
    IF (SELECT array_agg(e::int ORDER BY e::int) FROM jsonb_array_elements_text(got->'xs') e) <> '{1,2,100}'
        THEN RAISE EXCEPTION 'UNION inside: %', got; END IF;
    SELECT og_cypher('sub', $q$MATCH (s:Shelf {name: 'a'})
        CALL { WITH s MATCH (s)-[:HOLDS]->(b) RETURN b.v AS x UNION ALL WITH s MATCH (s)-[:HOLDS]->(b) RETURN b.v AS x }
        RETURN count(x) AS n$q$) INTO got;
    IF got <> '{"n": 4}' THEN RAISE EXCEPTION 'UNION ALL inside: %', got; END IF;
    SELECT og_cypher('sub', $q$CALL { MATCH (b:Box) RETURN max(b.v) AS m } MATCH (b:Box {v: m}) RETURN b.v AS v$q$) INTO got;
    IF got <> '{"v": 5}' THEN RAISE EXCEPTION 'an uncorrelated subquery: %', got; END IF;
    -- A node returned by the subquery anchors a pattern after it, on its type.
    SELECT jsonb_agg(r->'v' ORDER BY (r->>'v')::int) FROM og_cypher('sub', $q$MATCH (s:Shelf {name: 'a'})
        CALL { WITH s MATCH (s)-[:HOLDS]->(b) RETURN b } MATCH (b)<-[:HOLDS]-(o) RETURN b.v AS v$q$) r INTO got;
    IF got <> '[1, 2]' THEN RAISE EXCEPTION 'a node out of a subquery: %', got; END IF;
    -- Nested subqueries.
    SELECT og_cypher('sub', $q$MATCH (s:Shelf {name: 'c'}) CALL { WITH s CALL { WITH s MATCH (s)-->(b) RETURN b.v AS v } RETURN v * 2 AS w }
        RETURN w$q$) INTO got;
    IF got <> '{"w": 10}' THEN RAISE EXCEPTION 'a nested subquery: %', got; END IF;
    -- What the subquery cannot do, said.
    failed := false;
    BEGIN PERFORM og_cypher('sub', $q$MATCH (s:Shelf) CALL { MATCH (b:Box) RETURN b AS s } RETURN s$q$);
    EXCEPTION WHEN OTHERS THEN failed := SQLERRM LIKE '%already defined outside it%'; END;
    IF NOT failed THEN RAISE EXCEPTION 'a returned name shadowing an outer one was not refused'; END IF;
    failed := false;
    BEGIN PERFORM og_cypher('sub', $q$CALL (nope) { RETURN 1 AS one } RETURN one$q$);
    EXCEPTION WHEN OTHERS THEN failed := SQLERRM LIKE '%imports `nope`, which is not defined%'; END;
    IF NOT failed THEN RAISE EXCEPTION 'importing an undefined name was not refused'; END IF;
    failed := false;
    BEGIN PERFORM og_cypher('sub', $q$MATCH (s:Shelf) CALL { WITH s MATCH (s)-->(b) } RETURN s$q$);
    EXCEPTION WHEN OTHERS THEN failed := SQLERRM LIKE '%that only reads must end with RETURN%'; END;
    IF NOT failed THEN RAISE EXCEPTION 'a reading subquery without RETURN was not refused'; END IF;
    failed := false;
    BEGIN PERFORM og_cypher('sub', $q$UNWIND [1] AS i CALL { WITH i CREATE (:Never) } IN TRANSACTIONS OF 10 ROWS$q$);
    EXCEPTION WHEN OTHERS THEN failed := SQLERRM LIKE '%IN TRANSACTIONS is not supported%'; END;
    IF NOT failed THEN RAISE EXCEPTION 'IN TRANSACTIONS was not refused'; END IF;
    SELECT og_cypher('sub', $q$MATCH (n:Never) RETURN count(n) AS c$q$) INTO got;
    IF got <> '{"c": 0}' THEN RAISE EXCEPTION 'a refused IN TRANSACTIONS wrote: %', got; END IF;

    ------------------------------------------------------------- writing subqueries
    SELECT og_cypher('sub', $q$MATCH (s:Shelf) CALL { WITH s CREATE (:Log {of: s.name}) } RETURN count(*) AS rows$q$) INTO got;
    IF got <> '{"rows": 3}' THEN RAISE EXCEPTION 'a unit subquery keeps the rows: %', got; END IF;
    SELECT og_cypher('sub', $q$MATCH (l:Log) RETURN count(l) AS c$q$) INTO got;
    IF got <> '{"c": 3}' THEN RAISE EXCEPTION 'a unit subquery wrote once per row: %', got; END IF;
    SELECT og_cypher('sub', $q$UNWIND [1, 2] AS i CALL { WITH i MERGE (t:Tally {k: i}) RETURN t.k * 10 AS tk } RETURN collect(tk) AS tks$q$) INTO got;
    IF got <> '{"tks": [10, 20]}' THEN RAISE EXCEPTION 'a writing subquery with RETURN: %', got; END IF;
    -- Each run sees the runs before it.
    SELECT og_cypher('sub', $q$UNWIND [1, 1, 1] AS i CALL { WITH i MERGE (t:Tally {k: i})
        ON MATCH SET t.hits = coalesce(t.hits, 0) + 1 RETURN t.hits AS h } RETURN collect(h) AS hs$q$) INTO got;
    IF got <> '{"hs": [1, 2, 3]}' THEN RAISE EXCEPTION 'runs in order, each seeing the last: %', got; END IF;
    SELECT og_cypher('sub', $q$UNWIND ['x', 'y', 'z'] AS k CALL { WITH k OPTIONAL MATCH (e:Entry) WITH k, count(e) AS before
        CREATE (:Entry {k: k}) RETURN before } RETURN collect(before) AS bs$q$) INTO got;
    IF got <> '{"bs": [0, 1, 2]}' THEN RAISE EXCEPTION 'a read inside a writing subquery: %', got; END IF;
    -- A writing body that returns nothing drops the row; under OPTIONAL it is kept.
    SELECT og_cypher('sub', $q$MATCH (s:Shelf) CALL { WITH s MATCH (s)-[:HOLDS]->(b) SET b.seen = true RETURN b.v AS v }
        RETURN count(*) AS rows, collect(DISTINCT s.name) AS shelves$q$) INTO got;
    IF got->>'rows' <> '3' OR jsonb_array_length(got->'shelves') <> 2 THEN RAISE EXCEPTION 'an empty writing body: %', got; END IF;
    SELECT og_cypher('sub', $q$MATCH (s:Shelf) OPTIONAL CALL { WITH s MATCH (s)-[:HOLDS]->(b) SET b.seen = true RETURN b.v AS v }
        RETURN count(*) AS rows, count(v) AS vs$q$) INTO got;
    IF got <> '{"rows": 4, "vs": 3}' THEN RAISE EXCEPTION 'OPTIONAL CALL over a writing body: %', got; END IF;
    -- What the body did to an imported node is what the query sees after it.
    SELECT og_cypher('sub', $q$MATCH (s:Shelf {name: 'b'}) CALL { WITH s SET s.checked = 1 } RETURN s.checked AS c$q$) INTO got;
    IF got <> '{"c": 1}' THEN RAISE EXCEPTION 'an imported node after the body wrote it: %', got; END IF;
    -- A write-then-read query with FOREACH and booleans across the boundary (Robo Architect's shape).
    SELECT og_cypher('sub', $q$UNWIND $items AS it MATCH (s:Shelf {name: it.shelf}) WITH s, it
        CALL { WITH it OPTIONAL MATCH (b:Box {v: it.v}) RETURN b AS box }
        WITH s, it, box, (box IS NOT NULL) AS found
        FOREACH (_ IN CASE WHEN found THEN [1] ELSE [] END | MERGE (s)-[:LISTS]->(box))
        RETURN count(*) AS scanned, sum(CASE WHEN found THEN 1 ELSE 0 END) AS linked,
               sum(CASE WHEN NOT found THEN 1 ELSE 0 END) AS missing$q$,
        '{"items": [{"shelf": "b", "v": 1}, {"shelf": "b", "v": 42}]}') INTO got;
    IF got <> '{"scanned": 2, "linked": 1, "missing": 1}' THEN RAISE EXCEPTION 'subquery + FOREACH + booleans: %', got; END IF;
    SELECT og_cypher('sub', $q$MATCH (:Shelf {name: 'b'})-[:LISTS]->(b:Box) RETURN collect(b.v) AS vs$q$) INTO got;
    IF got <> '{"vs": [1]}' THEN RAISE EXCEPTION 'FOREACH after a subquery wrote: %', got; END IF;

    ------------------------------------------------------------- functions
    SELECT og_cypher('sub', $q$RETURN range(1, 5) AS r, range(10, 0, -3) AS r2, range(1, 0) AS r3,
        left('hello', 2) AS l, right('hello', 3) AS rt, ltrim('  x ') AS lt, rtrim(' x  ') AS rtr,
        reverse('abc') AS rv, reverse([1, 2, 3]) AS rl, sign(-4.2) AS sg, sign(0) AS s0,
        nullIf(1, 1) AS n1, nullIf('a', 'b') AS n2, isEmpty([]) AS e1, isEmpty('x') AS e2, isEmpty({}) AS e3,
        toBoolean('TRUE') AS tb, toBoolean('nope') AS tn, toBoolean(0) AS tz$q$) INTO got;
    IF got <> '{"r": [1, 2, 3, 4, 5], "r2": [10, 7, 4, 1], "r3": [], "l": "he", "rt": "llo", "lt": "x ", "rtr": " x",
                "rv": "cba", "rl": [3, 2, 1], "sg": -1, "s0": 0, "n1": null, "n2": "a", "e1": true, "e2": false,
                "e3": true, "tb": true, "tn": null, "tz": false}'
        THEN RAISE EXCEPTION 'functions: %', got; END IF;
    SELECT og_cypher('sub', $q$MATCH (s:Shelf {name: 'a'}) UNWIND range(0, size(s.name) - 1) AS i RETURN collect(i) AS is$q$) INTO got;
    IF got <> '{"is": [0]}' THEN RAISE EXCEPTION 'range() over a property: %', got; END IF;
    -- The same functions in a write clause.
    SELECT og_cypher('sub', $q$MATCH (s:Shelf {name: 'a'}) SET s.r = range(1, 3), s.l = left(s.name, 1), s.sg = sign(-1),
        s.rv = reverse([1, 2]), s.n = nullIf(s.name, 'a'), s.e = isEmpty('') RETURN s.r AS r, s.l AS l, s.sg AS sg, s.rv AS rv,
        s.n AS n, s.e AS e$q$) INTO got;
    IF got <> '{"r": [1, 2, 3], "l": "a", "sg": -1, "rv": [2, 1], "n": null, "e": true}' THEN RAISE EXCEPTION 'functions in SET: %', got; END IF;
    failed := false;
    BEGIN PERFORM og_cypher('sub', $q$RETURN point({x: 1, y: 2}) AS p$q$);
    EXCEPTION WHEN OTHERS THEN failed := SQLERRM LIKE '%spatial values are not supported%'; END;
    IF NOT failed THEN RAISE EXCEPTION 'point() was not refused by name'; END IF;
    failed := false;
    BEGIN PERFORM og_cypher('sub', $q$RETURN range(1, 3, 0) AS z$q$);
    EXCEPTION WHEN OTHERS THEN failed := true; END;
    IF NOT failed THEN RAISE EXCEPTION 'range() with a zero step was not refused'; END IF;
END $$;
