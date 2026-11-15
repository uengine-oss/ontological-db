-- Neo4j 5 syntax that applications (an analyzer, Robo Architect) send and that
-- used to be refused at parse time or to fail in SQL:
--   EXISTS { … } / exists((a)-[:R]->()) / a bare pattern predicate, COUNT { … },
--   COLLECT { … RETURN x }, pattern comprehensions, FOREACH, reduce(),
--   startNode() / endNode(), randomUUID(), ORDER BY alias.prop,
--   `collect(a) + collect(b)` over a carried list, a CALL with no RETURN,
--   vector.similarity.cosine(), a YIELDed column against a parameter.
-- No expected errors: any error fails the file.
\set ON_ERROR_STOP on

SELECT og_create_graph('n5');

DO $$
DECLARE got jsonb; failed bool;
BEGIN
    PERFORM og_cypher('n5', $q$CREATE (u1:Story {name: 'u1'})-[:IMPLEMENTS]->(:Context {name: 'b1'}),
                                     (u2:Story {name: 'u2'}),
                                     (u3:Story {name: 'u3'})-[:IMPLEMENTS]->(:Aggregate {name: 'g1'})$q$);

    ------------------------------------------------------------- pattern predicates
    SELECT jsonb_agg(r->'n' ORDER BY r->>'n') FROM og_cypher('n5', $q$MATCH (u:Story) WHERE EXISTS { (u)-[:IMPLEMENTS]->(:Context) } RETURN u.name AS n$q$) r INTO got;
    IF got <> '["u1"]' THEN RAISE EXCEPTION 'EXISTS { pattern }: %', got; END IF;
    SELECT jsonb_agg(r->'n' ORDER BY r->>'n') FROM og_cypher('n5', $q$MATCH (u:Story) WHERE EXISTS { MATCH (u)-[:IMPLEMENTS]->(t) WHERE t.name STARTS WITH 'g' } RETURN u.name AS n$q$) r INTO got;
    IF got <> '["u3"]' THEN RAISE EXCEPTION 'EXISTS { MATCH … WHERE … }: %', got; END IF;
    SELECT jsonb_agg(r->'n' ORDER BY r->>'n') FROM og_cypher('n5', $q$MATCH (u:Story) WHERE NOT (u)-[:IMPLEMENTS]->(:Context) RETURN u.name AS n$q$) r INTO got;
    IF got <> '["u2", "u3"]' THEN RAISE EXCEPTION 'NOT (a)-[:R]->(:B): %', got; END IF;
    SELECT jsonb_agg(r->'n' ORDER BY r->>'n') FROM og_cypher('n5', $q$MATCH (u:Story) WHERE exists((u)-[:IMPLEMENTS]->()) RETURN u.name AS n$q$) r INTO got;
    IF got <> '["u1", "u3"]' THEN RAISE EXCEPTION 'exists((a)-[:R]->()): %', got; END IF;
    -- exists(n.prop) is still the property test.
    SELECT og_cypher('n5', $q$MATCH (u:Story) WHERE exists(u.name) RETURN count(u) AS c$q$) INTO got;
    IF got <> '{"c": 3}' THEN RAISE EXCEPTION 'exists(n.prop): %', got; END IF;
    -- The subquery's pattern does not leak bindings or rows into the query.
    SELECT og_cypher('n5', $q$MATCH (u:Story) WHERE (u)-[:IMPLEMENTS]->() RETURN count(*) AS c$q$) INTO got;
    IF got <> '{"c": 2}' THEN RAISE EXCEPTION 'a pattern predicate multiplied rows: %', got; END IF;

    ------------------------------------------------------------- COUNT { }, COLLECT { }, comprehensions
    SELECT jsonb_agg(r ORDER BY r->>'n') FROM og_cypher('n5', $q$MATCH (u:Story) RETURN u.name AS n, COUNT { (u)-[:IMPLEMENTS]->() } AS c,
                                COLLECT { MATCH (u)-[:IMPLEMENTS]->(t) RETURN t.name } AS ts,
                                [(u)-[:IMPLEMENTS]->(t) WHERE t.name <> 'zz' | t.name] AS pc$q$) r INTO got;
    IF got <> '[{"n": "u1", "c": 1, "ts": ["b1"], "pc": ["b1"]}, {"n": "u2", "c": 0, "ts": [], "pc": []},
                {"n": "u3", "c": 1, "ts": ["g1"], "pc": ["g1"]}]' THEN RAISE EXCEPTION 'COUNT / COLLECT / comprehension: %', got; END IF;
    -- A list literal starting with a parenthesis is still a list.
    SELECT og_cypher('n5', $q$RETURN [(1 + 2), 4] AS l, (1 + 2) * 2 AS x$q$) INTO got;
    IF got <> '{"l": [3, 4], "x": 6}' THEN RAISE EXCEPTION 'parenthesised list items: %', got; END IF;

    ------------------------------------------------------------- reduce
    SELECT og_cypher('n5', $q$RETURN reduce(t = 0, x IN [1, 2, 3] | t + x) AS s, reduce(s = '', x IN ['a', 'b'] | s + x) AS c,
                                reduce(l = [], x IN [1, 2] | l + [x * 10]) AS l, reduce(t = 0, x IN [] | t + x) AS e$q$) INTO got;
    IF got <> '{"s": 6, "c": "ab", "l": [10, 20], "e": 0}' THEN RAISE EXCEPTION 'reduce(): %', got; END IF;
    SELECT og_cypher('n5', $q$MATCH (u:Story) WITH collect(u.name) AS ns RETURN reduce(acc = '', n IN ns | acc + n) AS joined$q$) INTO got;
    IF length(got->>'joined') <> 6 THEN RAISE EXCEPTION 'reduce() over a collected list: %', got; END IF;

    ------------------------------------------------------------- functions
    SELECT og_cypher('n5', $q$MATCH (:Story)-[r:IMPLEMENTS]->(:Context) RETURN startNode(r).name AS s, endNode(r).name AS e$q$) INTO got;
    IF got <> '{"s": "u1", "e": "b1"}' THEN RAISE EXCEPTION 'startNode / endNode: %', got; END IF;
    SELECT og_cypher('n5', $q$RETURN size(randomUUID()) AS l, randomUUID() <> randomUUID() AS differ$q$) INTO got;
    IF got <> '{"l": 36, "differ": true}' THEN RAISE EXCEPTION 'randomUUID() in a read: %', got; END IF;
    SELECT og_cypher('n5', $q$MERGE (b:Bounded {key: 'k1'}) ON CREATE SET b.uid = randomUUID() RETURN b.uid =~ '[0-9a-f-]{36}' AS ok$q$) INTO got;
    IF got <> '{"ok": true}' THEN RAISE EXCEPTION 'randomUUID() in a write: %', got; END IF;
    SELECT jsonb_agg(r->'a'->'name') FROM og_cypher('n5', $q$MATCH (u:Story) RETURN u {.name, len: size(u.name)} AS a ORDER BY a.name DESC$q$) r INTO got;
    IF got <> '["u3", "u2", "u1"]' THEN RAISE EXCEPTION 'ORDER BY alias.prop: %', got; END IF;
    SELECT og_cypher('n5', $q$MATCH (u:Story) WITH collect(u.name) AS a MATCH (c:Context) RETURN size(a + collect(c.name)) AS n$q$) INTO got;
    IF got <> '{"n": 4}' THEN RAISE EXCEPTION 'collect() + carried list: %', got; END IF;
    SELECT og_cypher('n5', $q$MATCH (u:Story) WITH collect(u.name) AS a, collect(u.name) AS b RETURN a + b AS ab$q$) INTO got;
    IF jsonb_array_length(got->'ab') <> 6 THEN RAISE EXCEPTION 'carried list + carried list: %', got; END IF;
    SELECT og_cypher('n5', $q$RETURN vector.similarity.cosine([1, 0], [1, 0]) AS same, vector.similarity.cosine([1, 0], [-1, 0]) AS opposite$q$) INTO got;
    IF got <> '{"same": 1, "opposite": 0}' THEN RAISE EXCEPTION 'vector.similarity.cosine: %', got; END IF;

    ------------------------------------------------------------- CALL with no RETURN, YIELD against a parameter
    PERFORM og_cypher('n5', $q$CALL db.awaitIndexes(30)$q$);
    PERFORM og_cypher('n5', $q$CREATE INDEX story_name IF NOT EXISTS FOR (s:Story) ON (s.name)$q$);
    SELECT og_cypher('n5', $q$SHOW INDEXES YIELD name, state WHERE name = $n RETURN name, state$q$, '{"n": "story_name"}') INTO got;
    IF got <> '{"name": "story_name", "state": "ONLINE"}' THEN RAISE EXCEPTION 'SHOW INDEXES … WHERE name = $n: %', got; END IF;

    ------------------------------------------------------------- FOREACH
    PERFORM og_cypher('n5', $q$MATCH (u:Story {name: 'u1'})
                              FOREACH (x IN CASE WHEN u.name = 'u1' THEN [1] ELSE [] END | SET u.flag = true)
                              FOREACH (k IN ['t1', 't2'] | MERGE (:Label {name: k}))
                              FOREACH (k IN ['t1'] | MERGE (:Label {name: k}))$q$);
    SELECT og_cypher('n5', $q$MATCH (u:Story {name: 'u1'}), (l:Label) RETURN u.flag AS f, count(l) AS labels$q$) INTO got;
    IF got <> '{"f": true, "labels": 2}' THEN RAISE EXCEPTION 'FOREACH: %', got; END IF;
    PERFORM og_cypher('n5', $q$MATCH (u:Story {name: 'u2'}) FOREACH (x IN CASE WHEN false THEN [1] ELSE [] END | SET u.flag = true)$q$);
    SELECT og_cypher('n5', $q$MATCH (u:Story {name: 'u2'}) RETURN u.flag AS f$q$) INTO got;
    IF got <> '{"f": null}' THEN RAISE EXCEPTION 'FOREACH over an empty list wrote: %', got; END IF;
    -- Nested, with a relationship to the row's node, and the row's variable visible.
    PERFORM og_cypher('n5', $q$MATCH (u:Story {name: 'u3'})
                              FOREACH (g IN [['a', 'b'], ['c']] | FOREACH (n IN g | CREATE (u)-[:NOTE]->(:Note {text: n})))$q$);
    SELECT og_cypher('n5', $q$MATCH (:Story {name: 'u3'})-[:NOTE]->(n:Note) RETURN count(n) AS c$q$) INTO got;
    IF got <> '{"c": 3}' THEN RAISE EXCEPTION 'nested FOREACH: %', got; END IF;
    failed := false;
    BEGIN
        PERFORM og_cypher('n5', $q$FOREACH (x IN [1] | MATCH (n) SET n.y = 1)$q$);
    EXCEPTION WHEN OTHERS THEN failed := SQLERRM LIKE '%FOREACH may only hold writes%';
    END;
    IF NOT failed THEN RAISE EXCEPTION 'a read inside FOREACH was not refused'; END IF;
    -- A pattern predicate has no meaning in a write clause's value; said so.
    failed := false;
    BEGIN
        PERFORM og_cypher('n5', $q$MATCH (u:Story) SET u.has = exists((u)-->())$q$);
    EXCEPTION WHEN OTHERS THEN failed := SQLERRM LIKE '%cannot be evaluated in a write clause%';
    END;
    IF NOT failed THEN RAISE EXCEPTION 'a pattern in SET was not refused'; END IF;
    -- … but computed in a WITH before the write, it is fine.
    PERFORM og_cypher('n5', $q$MATCH (u:Story) WITH u, exists((u)-[:IMPLEMENTS]->()) AS has SET u.has = has$q$);
    SELECT og_cypher('n5', $q$MATCH (u:Story) WHERE u.has RETURN count(u) AS c$q$) INTO got;
    IF got <> '{"c": 2}' THEN RAISE EXCEPTION 'a pattern predicate through WITH into SET: %', got; END IF;
END $$;
