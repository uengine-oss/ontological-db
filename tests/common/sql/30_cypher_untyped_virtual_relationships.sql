-- A relationship pattern without a type, over virtual relationships.
--
-- `(c:Crate)-[r]->(d:Depot)` used to read only the stored adjacency, which no
-- virtual relationship is in, and returned nothing where `-[r:STORED_AT]->`
-- returned rows. A hop without one type is the union of what each type
-- matches, as in Neo4j: every virtual relationship whose ends fit the hop's
-- ends and direction, and the stored edges. `type(r)` is each row's own type;
-- `[r:A|B]` is the union of A and B. Where the hop cannot be joined — an
-- unlabelled end beside a virtual type, a variable length — it is an error,
-- not an empty result.
--
-- The source is this database again, through a loopback. The refusals are
-- checked inside the block, and no error is expected: any error fails the file.
\set ON_ERROR_STOP on

CREATE SCHEMA yard_src;
CREATE TABLE yard_src.depot (depot_id int PRIMARY KEY, name text, neighbor int);
CREATE TABLE yard_src.crate (crate_id int PRIMARY KEY, home_depot int, last_depot bigint, label text);
INSERT INTO yard_src.depot VALUES (1, 'North', 2), (2, 'South', 3), (3, 'East', NULL), (4, 'West', 1);
-- 11 has two relationships to depot 1; 12 and 13 one key each; 14's home is no depot.
INSERT INTO yard_src.crate VALUES (10, 1, 2, 'a'), (11, 1, 1, 'b'), (12, 2, NULL, 'c'),
                                  (13, NULL, 3, 'd'), (14, 9, 3, 'e');

SELECT og_create_graph('yard');
SELECT og_create_datasource('yardsrc', 'postgres', jsonb_build_object(
    'host', 'localhost', 'port', current_setting('port'), 'database', current_database(),
    'user', current_user, 'password', ''));
SELECT og_map_virtual('yard', 'Depot', 'yardsrc', 'yard_src.depot', 'depot_id');
SELECT og_map_virtual('yard', 'Crate', 'yardsrc', 'yard_src.crate', 'crate_id');
-- Two relationship types between the same two labels, keyed on columns of different types.
SELECT og_create_virtual_rel('yard', 'STORED_AT', 'Crate', 'home_depot', 'Depot', 'depot_id');
SELECT og_create_virtual_rel('yard', 'LAST_SEEN_AT', 'Crate', 'last_depot', 'Depot', 'depot_id');
-- One from a type to itself.
SELECT og_create_virtual_rel('yard', 'NEXT_TO', 'Depot', 'neighbor', 'Depot', 'depot_id');
-- A native type with a virtual relationship to Depot and stored edges onto both virtual types.
SELECT og_create_type('yard', 'Inspector', 'entity');
SELECT og_add_property('yard', 'Inspector', 'name', 'string');
SELECT og_add_property('yard', 'Inspector', 'depot_code', 'integer');
SELECT og_create_virtual_rel('yard', 'ASSIGNED_TO', 'Inspector', 'depot_code', 'Depot', 'depot_id');
SELECT og_cypher('yard', 'CREATE (:Inspector {name: "Ida", depot_code: 1}), (:Inspector {name: "Ole", depot_code: 3})');
SELECT og_cypher('yard', 'MATCH (i:Inspector {name: "Ida"}), (d:Depot {depot_id: 2}) CREATE (i)-[:VISITED {times: 2}]->(d)');
SELECT og_cypher('yard', 'MATCH (i:Inspector {name: "Ida"}), (c:Crate {crate_id: 10}) CREATE (i)-[:CHECKED]->(c)');
SELECT og_cypher('yard', 'MATCH (i:Inspector {name: "Ida"}), (o:Inspector {name: "Ole"}) CREATE (i)-[:MENTORS]->(o)');

DO $$
DECLARE got jsonb; typed jsonb; want jsonb; n int; failed bool;
BEGIN
    ------------------------------------------------------- untyped = union of typed
    SELECT jsonb_agg(r ORDER BY r->>'c', r->>'t') FROM og_cypher('yard',
        'MATCH (c:Crate)-[r]->(d:Depot) RETURN c.crate_id AS c, type(r) AS t, d.depot_id AS d, id(r) AS i') r INTO got;
    SELECT jsonb_agg(r ORDER BY r->>'c', r->>'t') FROM (
        SELECT r FROM og_cypher('yard', 'MATCH (c:Crate)-[r:STORED_AT]->(d:Depot)
                                          RETURN c.crate_id AS c, type(r) AS t, d.depot_id AS d, id(r) AS i') r
        UNION ALL
        SELECT r FROM og_cypher('yard', 'MATCH (c:Crate)-[r:LAST_SEEN_AT]->(d:Depot)
                                          RETURN c.crate_id AS c, type(r) AS t, d.depot_id AS d, id(r) AS i') r) u INTO want;
    IF jsonb_array_length(want) <> 7 THEN RAISE EXCEPTION 'fixture: 3 + 4 typed rows expected, got %', want; END IF;
    IF got IS DISTINCT FROM want THEN
        RAISE EXCEPTION 'untyped hop should be the union of the typed ones (same rows, types, ids): % vs %', got, want;
    END IF;
    -- Two types between the same pair: one row each, each with its own type.
    SELECT jsonb_agg(r->>'t' ORDER BY r->>'t') FROM og_cypher('yard',
        'MATCH (c:Crate {crate_id: 11})-[r]->(d:Depot {depot_id: 1}) RETURN type(r) AS t') r INTO got;
    IF got <> '["LAST_SEEN_AT", "STORED_AT"]' THEN RAISE EXCEPTION 'two types between one pair: %', got; END IF;
    SELECT og_cypher('yard', 'MATCH (c:Crate {crate_id: 10})-[r]->(d:Depot {depot_id: 1}) RETURN r') INTO got;
    IF got->'r'->>'_type' <> 'STORED_AT' OR got->'r'->'_src' IS NULL OR got->'r'->'_dst' IS NULL THEN
        RAISE EXCEPTION 'relationship value: %', got;
    END IF;
    SELECT og_cypher('yard', 'MATCH (c:Crate)-->(d:Depot) RETURN count(*) AS n') INTO got;
    IF got <> '{"n": 7}' THEN RAISE EXCEPTION 'anonymous untyped hop: %', got; END IF;
    SELECT jsonb_agg(r ORDER BY r->>'t') FROM og_cypher('yard',
        'MATCH (c:Crate)-[r]->(d:Depot) RETURN type(r) AS t, count(*) AS n') r INTO got;
    IF got <> '[{"n": 4, "t": "LAST_SEEN_AT"}, {"n": 3, "t": "STORED_AT"}]' THEN RAISE EXCEPTION 'grouped by type(r): %', got; END IF;

    ------------------------------------------------------- direction
    SELECT og_cypher('yard', 'MATCH (d:Depot)<-[r]-(c:Crate) RETURN count(*) AS n') INTO got;
    IF got <> '{"n": 7}' THEN RAISE EXCEPTION 'incoming, written from the other end: %', got; END IF;
    SELECT og_cypher('yard', 'MATCH (c:Crate)<-[r]-(d:Depot) RETURN count(*) AS n') INTO got;
    IF got <> '{"n": 0}' THEN RAISE EXCEPTION 'against the relationships'' direction nothing matches: %', got; END IF;
    SELECT og_cypher('yard', 'MATCH (c:Crate)-[r]-(d:Depot) RETURN count(*) AS n') INTO got;
    IF got <> '{"n": 7}' THEN RAISE EXCEPTION 'undirected: %', got; END IF;
    SELECT og_cypher('yard', 'MATCH (d:Depot)-[r]-(c:Crate) RETURN count(*) AS n') INTO got;
    IF got <> '{"n": 7}' THEN RAISE EXCEPTION 'undirected, from the other end: %', got; END IF;
    -- A relationship from a type to itself reads as its typed hop does, either way.
    FOREACH want IN ARRAY ARRAY['["-[r]-", "-[r:NEXT_TO]-"]', '["-[r]->", "-[r:NEXT_TO]->"]', '["<-[r]-", "<-[r:NEXT_TO]-"]']::jsonb[] LOOP
        SELECT og_cypher('yard', format('MATCH (a:Depot)%s(b:Depot) RETURN count(*) AS n', want->>0)) INTO got;
        SELECT og_cypher('yard', format('MATCH (a:Depot)%s(b:Depot) RETURN count(*) AS n', want->>1)) INTO typed;
        IF got <> typed OR got <> '{"n": 3}' THEN
            RAISE EXCEPTION 'self relationship %: % (typed: %)', want->>0, got, typed;
        END IF;
    END LOOP;

    ------------------------------------------------------- type alternatives
    SELECT og_cypher('yard', 'MATCH (c:Crate)-[r:STORED_AT|LAST_SEEN_AT]->(d:Depot) RETURN count(*) AS n') INTO got;
    IF got <> '{"n": 7}' THEN RAISE EXCEPTION '[r:A|B] over two virtual types: %', got; END IF;
    SELECT og_cypher('yard', 'MATCH (c:Crate)-[r:STORED_AT|NEXT_TO|NOPE]->(d:Depot) RETURN count(*) AS n') INTO got;
    IF got <> '{"n": 3}' THEN RAISE EXCEPTION '[r:A|B] where one fits the ends: %', got; END IF;
    -- Every type named is virtual and they agree on the unlabelled end: it takes that type.
    SELECT og_cypher('yard', 'MATCH (c:Crate)-[r:STORED_AT|LAST_SEEN_AT]->(d) RETURN count(DISTINCT d.name) AS n') INTO got;
    IF got <> '{"n": 3}' THEN RAISE EXCEPTION '[r:A|B] with an unlabelled end: %', got; END IF;

    ------------------------------------------------------- stored and virtual together
    SELECT jsonb_agg(r ORDER BY r->>'i', r->>'t') FROM og_cypher('yard',
        'MATCH (i:Inspector)-[r]->(d:Depot) RETURN i.name AS i, type(r) AS t, d.name AS d, r.times AS times') r INTO got;
    IF got <> '[{"d": "North", "i": "Ida", "t": "ASSIGNED_TO", "times": null}, {"d": "South", "i": "Ida", "t": "VISITED", "times": 2},
                {"d": "East", "i": "Ole", "t": "ASSIGNED_TO", "times": null}]' THEN
        RAISE EXCEPTION 'a stored edge and a virtual relationship between the same labels: %', got;
    END IF;
    SELECT og_cypher('yard', 'MATCH (i:Inspector)-[r:VISITED|ASSIGNED_TO]-(d:Depot) RETURN count(*) AS n') INTO got;
    IF got <> '{"n": 3}' THEN RAISE EXCEPTION '[r:Stored|Virtual]: %', got; END IF;
    SELECT og_cypher('yard', 'MATCH (i:Inspector)-[r]->(d:Depot) WHERE r.times = 2 RETURN type(r) AS t') INTO got;
    IF got <> '{"t": "VISITED"}' THEN RAISE EXCEPTION 'a property of the stored alternative: %', got; END IF;
    SELECT og_cypher('yard', 'MATCH (c:Crate {crate_id: 10})-[r]-(i:Inspector) RETURN type(r) AS t') INTO got;
    IF got <> '{"t": "CHECKED"}' THEN RAISE EXCEPTION 'a stored edge onto a virtual node: %', got; END IF;
    SELECT jsonb_agg(r ORDER BY r->>'i', r->>'t') FROM og_cypher('yard',
        'MATCH (i:Inspector)-[r]->(d:Depot)-[s]->(e:Depot) RETURN i.name AS i, type(r) AS t, type(s) AS s, e.depot_id AS e') r INTO got;
    IF got <> '[{"e": 2, "i": "Ida", "s": "NEXT_TO", "t": "ASSIGNED_TO"}, {"e": 3, "i": "Ida", "s": "NEXT_TO", "t": "VISITED"}]' THEN
        RAISE EXCEPTION 'two untyped hops in a row: %', got;
    END IF;
    -- No virtual relationship touches a pattern without labels: it reads the stored graph, as before.
    SELECT og_cypher('yard', 'MATCH (a)-[r]->(b) RETURN collect(type(r)) AS ts') INTO got;
    IF got <> '{"ts": ["MENTORS"]}' THEN RAISE EXCEPTION 'an unlabelled pattern: %', got; END IF;

    ------------------------------------------------------- OPTIONAL MATCH, paths, subqueries, isomorphism
    SELECT jsonb_agg(r ORDER BY (r->>'id')::int) FROM og_cypher('yard',
        'MATCH (d:Depot) OPTIONAL MATCH (d)<-[r]-(c:Crate) RETURN d.depot_id AS id, count(r) AS n') r INTO got;
    IF got <> '[{"n": 3, "id": 1}, {"n": 2, "id": 2}, {"n": 2, "id": 3}, {"n": 0, "id": 4}]' THEN
        RAISE EXCEPTION 'OPTIONAL MATCH counts: %', got;
    END IF;
    SELECT count(*) INTO n FROM og_cypher('yard', 'MATCH (d:Depot) OPTIONAL MATCH (d)<-[r]-(c:Crate) RETURN d, r');
    IF n <> 8 THEN RAISE EXCEPTION 'OPTIONAL MATCH: 7 rows and one of NULLs expected, got %', n; END IF;
    SELECT jsonb_agg(r ORDER BY (r->>'c')::int, r->>'t') FROM og_cypher('yard',
        'MATCH (c:Crate) OPTIONAL MATCH (c)-[r]->(d:Depot) WHERE d.name = ''North'' RETURN c.crate_id AS c, type(r) AS t') r INTO got;
    IF got <> '[{"c": 10, "t": "STORED_AT"}, {"c": 11, "t": "LAST_SEEN_AT"}, {"c": 11, "t": "STORED_AT"},
                {"c": 12, "t": null}, {"c": 13, "t": null}, {"c": 14, "t": null}]' THEN
        RAISE EXCEPTION 'OPTIONAL MATCH … WHERE: one row of NULLs per unmatched crate: %', got;
    END IF;
    SELECT og_cypher('yard', 'MATCH (c:Crate {crate_id: 12}), (d:Depot {depot_id: 1}) OPTIONAL MATCH (c)-[r]->(d) RETURN r') INTO got;
    IF got <> '{"r": null}' THEN RAISE EXCEPTION 'OPTIONAL MATCH between bound nodes: %', got; END IF;
    SELECT og_cypher('yard', 'MATCH (c:Crate {crate_id: 11}), (d:Depot {depot_id: 1}) MATCH (c)-[r]->(d) RETURN count(*) AS n') INTO got;
    IF got <> '{"n": 2}' THEN RAISE EXCEPTION 'a hop between bound nodes: %', got; END IF;
    SELECT og_cypher('yard', 'MATCH (d:Depot {depot_id: 1}) WITH d MATCH (d)<-[r]-(c:Crate) RETURN count(*) AS n') INTO got;
    IF got <> '{"n": 3}' THEN RAISE EXCEPTION 'a node carried through WITH: %', got; END IF;
    SELECT jsonb_agg(r ORDER BY r->'ts') FROM og_cypher('yard',
        'MATCH p = (c:Crate {crate_id: 11})-[r]->(d:Depot) RETURN length(p) AS l, [x IN relationships(p) | x._type] AS ts') r INTO got;
    IF got <> '[{"l": 1, "ts": ["LAST_SEEN_AT"]}, {"l": 1, "ts": ["STORED_AT"]}]' THEN RAISE EXCEPTION 'path variable: %', got; END IF;
    SELECT og_cypher('yard', 'MATCH (d:Depot) WHERE NOT (d)<--(:Crate) RETURN collect(d.depot_id) AS ds') INTO got;
    IF got <> '{"ds": [4]}' THEN RAISE EXCEPTION 'pattern predicate: %', got; END IF;
    SELECT jsonb_agg(r ORDER BY (r->>'id')::int) FROM og_cypher('yard',
        'MATCH (d:Depot) RETURN d.depot_id AS id, COUNT { (d)<-[]-(:Crate) } AS n') r INTO got;
    IF got <> '[{"n": 3, "id": 1}, {"n": 2, "id": 2}, {"n": 2, "id": 3}, {"n": 0, "id": 4}]' THEN
        RAISE EXCEPTION 'COUNT subquery: %', got;
    END IF;
    -- One MATCH never uses a relationship twice: crate 11 reaches depot 1 twice,
    -- and depot 1 has three relationships in, so 2 × 2.
    SELECT og_cypher('yard', 'MATCH (a:Crate {crate_id: 11})-[r]->(d:Depot)<-[s]-(b:Crate) RETURN count(*) AS n') INTO got;
    IF got <> '{"n": 4}' THEN RAISE EXCEPTION 'isomorphism over untyped hops: %', got; END IF;

    ------------------------------------------------------- what cannot be joined is said
    failed := false;
    BEGIN PERFORM og_cypher('yard', 'MATCH (c:Crate)-[r]->(x) RETURN count(*) AS n');
    EXCEPTION WHEN OTHERS THEN failed := SQLERRM LIKE '%virtual node type ''Crate'' needs both ends labelled%'
                                      AND SQLERRM LIKE '%''STORED_AT'', ''LAST_SEEN_AT''%'; END;
    IF NOT failed THEN RAISE EXCEPTION 'an untyped hop from a virtual node to an unlabelled one was not refused'; END IF;
    failed := false;
    BEGIN PERFORM og_cypher('yard', 'MATCH (x)-[r]-(d:Depot) RETURN count(*) AS n');
    EXCEPTION WHEN OTHERS THEN failed := SQLERRM LIKE '%needs both ends labelled%'; END;
    IF NOT failed THEN RAISE EXCEPTION 'an undirected untyped hop to an unlabelled node was not refused'; END IF;
    failed := false;
    BEGIN PERFORM og_cypher('yard', 'MATCH (x)-[r:STORED_AT|ASSIGNED_TO]->(d:Depot) RETURN count(*) AS n');
    EXCEPTION WHEN OTHERS THEN failed := SQLERRM LIKE '%need both ends labelled%'; END;
    IF NOT failed THEN RAISE EXCEPTION '[r:A|B] whose sources differ, from an unlabelled node, was not refused'; END IF;
    failed := false;
    BEGIN PERFORM og_cypher('yard', 'MATCH (c:Crate)-[*1..2]->(d:Depot) RETURN count(*) AS n');
    EXCEPTION WHEN OTHERS THEN failed := SQLERRM LIKE '%cannot be traversed with a variable length yet%'; END;
    IF NOT failed THEN RAISE EXCEPTION 'an untyped variable-length hop at a virtual node was not refused'; END IF;
    failed := false;
    BEGIN PERFORM og_cypher('yard', 'MATCH (c:Crate)-[:STORED_AT|LAST_SEEN_AT*1..2]->(d:Depot) RETURN count(*) AS n');
    EXCEPTION WHEN OTHERS THEN failed := SQLERRM LIKE '%cannot be traversed with a variable length yet%'; END;
    IF NOT failed THEN RAISE EXCEPTION '[:A|B*] over virtual types was not refused'; END IF;
    failed := false;
    BEGIN PERFORM og_cypher('yard', 'MATCH (c:Crate)-[r:STORED_AT|LAST_SEEN_AT {x: 1}]->(d:Depot) RETURN count(*) AS n');
    EXCEPTION WHEN OTHERS THEN failed := SQLERRM LIKE '%has no properties to match%'; END;
    IF NOT failed THEN RAISE EXCEPTION 'a property filter on virtual types was not refused'; END IF;
    -- No type and a property filter: only a stored edge has properties, and none of these does.
    SELECT og_cypher('yard', 'MATCH (c:Crate)-[r {x: 1}]->(d:Depot) RETURN count(*) AS n') INTO got;
    IF got <> '{"n": 0}' THEN RAISE EXCEPTION 'an untyped hop with properties: %', got; END IF;
END
$$;
