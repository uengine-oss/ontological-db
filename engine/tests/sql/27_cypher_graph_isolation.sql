-- Found reviewing 0012: what a query can reach outside its own graph, and three
-- Neo4j functions that answered differently.
--
--   * A map parameter with an `_id` key (`WITH $m AS n SET n.x = 1`, `$m = {_id: …}`)
--     reached the node that number names in any graph: SET wrote to it, CREATE
--     hung a relationship off it, DETACH DELETE removed it, MATCH (n)-->(x) walked
--     from it. Reads and writes are now confined to the query's graph.
--   * `WHERE id(n) = $id` failed: "operator does not exist: bigint = text".
--   * keys(n) listed `_id` and `_type`; properties(r) kept `_src` / `_dst`.
--   * a duplicate key inside one UNWIND batch under a unique constraint, a
--     relationship pattern without labels, DETACH DELETE of a pattern, and a read
--     after a write that must see it.
\set ON_ERROR_STOP on

SELECT og_create_graph('iso');
SELECT og_create_graph('iso_far');

DO $$
DECLARE got jsonb; far jsonb; failed bool; n int;
BEGIN
    PERFORM og_cypher('iso_far', $q$CREATE (:Far {name: 'f1'})-[:LINK]->(:Far {name: 'f2'})$q$);
    SELECT og_cypher('iso_far', $q$MATCH (n:Far {name: 'f1'}) RETURN id(n) AS i$q$) INTO got;
    far := jsonb_build_object('m', jsonb_build_object('_id', got->'i'), 'i', got->'i');
    PERFORM og_cypher('iso', $q$CREATE (:Near {name: 'n1'})$q$);

    ------------------------------------------------------------- reads stay in the graph
    SELECT og_cypher('iso', $q$MATCH (n) WHERE id(n) = $i RETURN count(n) AS c$q$, far) INTO got;
    IF got <> '{"c": 0}' THEN RAISE EXCEPTION 'id(n) = another graph''s id: %', got; END IF;
    SELECT og_cypher('iso', $q$WITH $m AS n MATCH (n)-[r]->(x) RETURN count(x) AS c$q$, far) INTO got;
    IF got <> '{"c": 0}' THEN RAISE EXCEPTION 'a map parameter walked into another graph: %', got; END IF;
    SELECT og_cypher('iso', $q$UNWIND [$m] AS n MATCH (n) RETURN count(n) AS c$q$, far) INTO got;
    IF got <> '{"c": 0}' THEN RAISE EXCEPTION 'an unwound map matched another graph''s node: %', got; END IF;

    ------------------------------------------------------------- writes stay in the graph
    failed := false;
    BEGIN
        PERFORM og_cypher('iso', $q$WITH $m AS n SET n.hacked = true$q$, far);
    EXCEPTION WHEN OTHERS THEN failed := SQLERRM LIKE 'SET: % is not a node or relationship of this graph%';
    END;
    IF NOT failed THEN RAISE EXCEPTION 'SET through a map reached another graph'; END IF;
    failed := false;
    BEGIN
        PERFORM og_cypher('iso', $q$WITH $m AS n CREATE (n)-[:X]->(:Near)$q$, far);
    EXCEPTION WHEN OTHERS THEN failed := SQLERRM LIKE 'CREATE: % is not a node or relationship of this graph%';
    END;
    IF NOT failed THEN RAISE EXCEPTION 'CREATE through a map reached another graph'; END IF;
    failed := false;
    BEGIN
        PERFORM og_cypher('iso', $q$WITH $m AS n DETACH DELETE n$q$, far);
    EXCEPTION WHEN OTHERS THEN failed := SQLERRM LIKE 'DELETE: % is not a node or relationship of this graph%';
    END;
    IF NOT failed THEN RAISE EXCEPTION 'DETACH DELETE through a map reached another graph'; END IF;
    failed := false;
    BEGIN
        PERFORM og_cypher('iso', $q$WITH $m AS a MATCH (b:Near) MERGE (a)-[:Y]->(b)$q$, far);
    EXCEPTION WHEN OTHERS THEN failed := SQLERRM LIKE '% is not a node or relationship of this graph%';
    END;
    IF NOT failed THEN RAISE EXCEPTION 'MERGE through a map reached another graph'; END IF;
    SELECT og_cypher('iso_far', $q$MATCH (n:Far) RETURN count(n) AS c, collect(n.hacked) AS h$q$) INTO got;
    IF got <> '{"c": 2, "h": []}' THEN RAISE EXCEPTION 'the other graph was changed: %', got; END IF;
    SELECT og_cypher('iso_far', $q$MATCH (:Far)-[r]->(:Far) RETURN count(r) AS c$q$) INTO got;
    IF got <> '{"c": 1}' THEN RAISE EXCEPTION 'the other graph''s relationship was changed: %', got; END IF;
    -- Within the graph a map with its own node's id still works (as before).
    PERFORM og_cypher('iso', $q$MATCH (n:Near) WITH n {.*, _id: id(n)} AS m WITH m AS n SET n.touched = true$q$);
    SELECT og_cypher('iso', $q$MATCH (n:Near) RETURN n.touched AS t$q$) INTO got;
    IF got <> '{"t": true}' THEN RAISE EXCEPTION 'a same-graph map target: %', got; END IF;

    ------------------------------------------------------------- unlabelled relationships and DETACH DELETE
    PERFORM og_cypher('iso', $q$MATCH (a:Near) CREATE (a)-[:OWN]->(:Near {name: 'n2'})$q$);
    SELECT og_cypher('iso', $q$MATCH ()-[r]->() RETURN count(r) AS c$q$) INTO got;
    IF got <> '{"c": 1}' THEN RAISE EXCEPTION '()-[r]->() across graphs: %', got; END IF;
    SELECT og_cypher('iso', $q$MATCH ()-[r]-() RETURN count(r) AS c$q$) INTO got;
    IF got <> '{"c": 2}' THEN RAISE EXCEPTION '()-[r]-() across graphs: %', got; END IF;
    PERFORM og_cypher('iso', $q$MATCH (a)-[r]->(b) DETACH DELETE b$q$);
    SELECT og_cypher('iso', $q$MATCH (n) RETURN count(n) AS c$q$) INTO got;
    IF got <> '{"c": 1}' THEN RAISE EXCEPTION 'DETACH DELETE of a pattern in this graph: %', got; END IF;
    SELECT og_cypher('iso_far', $q$MATCH (a)-[r]->(b) RETURN count(r) AS c$q$) INTO got;
    IF got <> '{"c": 1}' THEN RAISE EXCEPTION 'DETACH DELETE of a pattern reached another graph: %', got; END IF;

    ------------------------------------------------------------- id(n) against a parameter, keys(), properties()
    SELECT og_cypher('iso', $q$MATCH (n:Near) RETURN id(n) AS i$q$) INTO got;
    SELECT og_cypher('iso', $q$MATCH (n) WHERE id(n) = $i RETURN n.name AS name$q$, got) INTO got;
    IF got <> '{"name": "n1"}' THEN RAISE EXCEPTION 'id(n) = $i: %', got; END IF;
    SELECT og_cypher('iso_far', $q$MATCH (a:Far)-[r:LINK]->() SET r.w = 3
                                    RETURN keys(a) AS ka, keys(r) AS kr, properties(r) AS pr, keys({_k: 1}) AS km$q$) INTO got;
    IF got <> '{"ka": ["name"], "kr": ["w"], "pr": {"w": 3}, "km": ["_k"]}' THEN RAISE EXCEPTION 'keys() / properties(): %', got; END IF;
    SELECT og_cypher('iso_far', $q$MATCH (a:Far)-[r:LINK]->() RETURN keys(a) AS ka, keys(r) AS kr, properties(r) AS pr$q$) INTO got;
    IF got <> '{"ka": ["name"], "kr": ["w"], "pr": {"w": 3}}' THEN RAISE EXCEPTION 'keys() / properties() in a read: %', got; END IF;

    ------------------------------------------------------------- a unique key twice in one batch, a read that sees a write
    PERFORM og_cypher('iso', $q$CREATE CONSTRAINT near_code IF NOT EXISTS FOR (n:Coded) REQUIRE n.id IS UNIQUE$q$);
    SELECT og_cypher('iso', $q$UNWIND ['c1', 'c1', 'c2', 'c1'] AS k MERGE (n:Coded {id: k}) ON MATCH SET n.again = true RETURN count(*) AS rows$q$) INTO got;
    IF got <> '{"rows": 4}' THEN RAISE EXCEPTION 'MERGE rows: %', got; END IF;
    SELECT og_cypher('iso', $q$MATCH (n:Coded) RETURN count(n) AS c, collect(n.again) AS a$q$) INTO got;
    IF (got->>'c')::int <> 2 OR got->'a' <> '[true]' THEN RAISE EXCEPTION 'a repeated key in one batch: %', got; END IF;
    failed := false;
    BEGIN
        PERFORM og_cypher('iso', $q$CREATE (:Coded {id: 'c2'})$q$);
    EXCEPTION WHEN unique_violation THEN failed := true;
    END;
    IF NOT failed THEN RAISE EXCEPTION 'the unique constraint on id did not hold'; END IF;
    SELECT og_cypher('iso', $q$UNWIND [1, 2] AS i CREATE (s:Seen {k: i}) WITH collect(s) AS made
                                MATCH (t:Seen) RETURN size(made) AS m, count(t) AS c$q$) INTO got;
    IF got <> '{"m": 2, "c": 2}' THEN RAISE EXCEPTION 'a read after a write: %', got; END IF;
    SELECT og_cypher('iso', $q$MATCH (s:Seen) SET s.v = s.k * 10 WITH s WHERE s.v > 10 RETURN collect(s.v) AS vs$q$) INTO got;
    IF got <> '{"vs": [20]}' THEN RAISE EXCEPTION 'WITH … WHERE after SET: %', got; END IF;
    SELECT og_cypher('iso', $q$MATCH (s:Seen {k: 1}) DETACH DELETE s WITH count(*) AS gone MATCH (t:Seen) RETURN gone, count(t) AS left$q$) INTO got;
    IF got <> '{"gone": 1, "left": 1}' THEN RAISE EXCEPTION 'a read after DETACH DELETE: %', got; END IF;
    -- Booleans carried across a write are still booleans (CASE WHEN, AND).
    SELECT og_cypher('iso', $q$MATCH (s:Seen) WITH s, s.k = 2 AS two, true AS yes SET s.flag = two
                                RETURN sum(CASE WHEN two AND yes THEN 1 ELSE 0 END) AS n$q$) INTO got;
    IF got <> '{"n": 1}' THEN RAISE EXCEPTION 'booleans across a write: %', got; END IF;
END $$;
