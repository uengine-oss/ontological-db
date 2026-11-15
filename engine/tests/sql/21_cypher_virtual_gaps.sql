-- Cypher an answering agent writes over virtual nodes, that used to fail.
--
--   * arithmetic inside an aggregate over values carried through WITH —
--     `function sum(jsonb) does not exist`, `operator does not exist: text * text`;
--   * list slices `xs[a..b]`, `xs[..b]`, `xs[a..]`, and head() / last() / tail();
--   * label predicates in an expression — `WHERE n:A OR n:B`, `NOT n:A`;
--   * a virtual hop whose end has no label: undirected, or a node carried
--     through WITH — "needs both ends labelled";
--   * a boolean compared with 0 or 1 — `operator does not exist: boolean = integer`;
--   * a source key column named `id`: `n.id` read the derived node identifier,
--     so `{id: 1}` and every virtual relationship joining on it found nothing.
-- Each is checked on stored nodes too. No expected errors: any error fails the file.
\set ON_ERROR_STOP on

CREATE SCHEMA src21;
CREATE TABLE src21.member (member_id int PRIMARY KEY, name text, active boolean, flag smallint);
CREATE TABLE src21.loan (loan_id int PRIMARY KEY, member_id int, days int,
                         fee numeric(6,2), discount float8, returned boolean, opened date);
CREATE TABLE src21.branch (id int PRIMARY KEY, city text);
CREATE TABLE src21.desk (id int PRIMARY KEY, branch_id int);
INSERT INTO src21.member VALUES (1, 'Ana', true, 1), (2, 'Ben', false, 0), (3, 'Cho', NULL, NULL);
INSERT INTO src21.branch VALUES (1, 'North'), (2, 'South');
INSERT INTO src21.desk VALUES (7, 1), (8, 1), (9, 2);
INSERT INTO src21.loan VALUES
    (10, 1, 2, 5.50, 0.5,  true,  '2024-01-05'),
    (11, 1, 3, 2.00, NULL, false, '2024-02-01'),
    (12, 2, 1, 10.00, 1.25, true, '2024-03-10'),
    (13, 2, NULL, 4.00, 0,  NULL,  '2024-03-11'),
    (14, 3, 4, 1.25, 0.25, true,  '2024-04-01');

SELECT og_create_graph('gaps');
SELECT og_create_datasource('loop21', 'postgres', jsonb_build_object(
    'host', 'localhost', 'port', current_setting('port'), 'database', current_database(),
    'user', current_user, 'password', ''));
SELECT og_map_virtual('gaps', 'Member', 'loop21', 'src21.member', 'member_id');
SELECT og_map_virtual('gaps', 'Loan', 'loop21', 'src21.loan', 'loan_id');
SELECT og_create_virtual_rel('gaps', 'BORROWED_BY', 'Loan', 'member_id', 'Member', 'member_id');
SELECT og_map_virtual('gaps', 'Branch', 'loop21', 'src21.branch', 'id');
SELECT og_map_virtual('gaps', 'Desk', 'loop21', 'src21.desk', 'id');
SELECT og_create_virtual_rel('gaps', 'AT_BRANCH', 'Desk', 'branch_id', 'Branch', 'id');

-- Stored twins, with a small hierarchy and an umbrella over stored classes.
SELECT og_create_type('gaps', '_Entity', 'entity', '{}', true);
SELECT og_create_type('gaps', 'Asset', 'entity', ARRAY['_Entity']);
SELECT og_create_type('gaps', 'Shelf', 'entity', ARRAY['Asset']);
SELECT og_cypher('gaps', 'CREATE (:Shelf {code: ''S1'', slots: 2, width: 1.5, open: true})');
SELECT og_cypher('gaps', 'CREATE (:Shelf {code: ''S2'', slots: 3, width: 2, open: false})');
SELECT og_cypher('gaps', 'CREATE (:Shelf {code: ''S3'', width: 4, open: true})');
SELECT og_cypher('gaps', 'CREATE (:Asset {code: ''A1'', slots: 1, width: 1, open: true})');

DO $$
DECLARE got jsonb; n int; failed boolean; sql text;
BEGIN
    ---------------------------------------------------------------- 1. arithmetic in aggregates
    -- Typed columns: int × numeric, int − float, NULLs skipped.
    SELECT og_cypher('gaps', 'MATCH (l:Loan) RETURN sum(l.days * l.fee) AS s, avg(l.days - l.discount) AS a') INTO got;
    IF (got->>'s')::numeric <> 32 OR round((got->>'a')::numeric, 4) <> 1.6667 THEN
        RAISE EXCEPTION 'aggregate over typed columns: %', got;
    END IF;
    -- A computed value carried through WITH is jsonb: sum/avg/min/max read it as a number.
    SELECT og_cypher('gaps', 'MATCH (l:Loan) WITH l.days * l.fee AS v
                              RETURN sum(v) AS s, avg(v) AS a, min(v) AS lo, max(v) AS hi') INTO got;
    IF (got->>'s')::numeric <> 32 OR (got->>'a')::numeric <> 8 OR (got->>'lo')::numeric <> 5 OR (got->>'hi')::numeric <> 11 THEN
        RAISE EXCEPTION 'aggregates of a value carried through WITH: %', got;
    END IF;
    -- A node carried through WITH: its properties keep their types.
    SELECT og_cypher('gaps', 'MATCH (l:Loan) WITH l RETURN sum(l.days * l.fee) AS s, max(l.opened) AS d') INTO got;
    IF (got->>'s')::numeric <> 32 OR got->>'d' <> '2024-04-01' THEN
        RAISE EXCEPTION 'arithmetic on a node carried through WITH: %', got;
    END IF;
    -- toFloat() inside, and a grouping key beside it.
    SELECT og_cypher('gaps', 'MATCH (l:Loan)-[:BORROWED_BY]->(m:Member) WITH m, toFloat(l.days) * l.discount AS v
                              RETURN m.name AS name, sum(v) AS s ORDER BY name LIMIT 1') INTO got;
    IF got->>'name' <> 'Ana' OR (got->>'s')::numeric <> 1 THEN
        RAISE EXCEPTION 'toFloat() through WITH, grouped: %', got;
    END IF;
    -- Values of unknown type (UNWIND): integers divide as integers, the rest exactly.
    SELECT og_cypher('gaps', 'UNWIND [7, 3, null, 2.5] AS x
                              RETURN sum(x * 2) AS s, sum(x / 2) AS d, min(x) AS lo, max(x) AS hi, sum(-x) AS ng') INTO got;
    IF got <> '{"s": 25.0, "d": 5.25, "lo": 2.5, "hi": 7, "ng": -12.5}'::jsonb THEN
        RAISE EXCEPTION 'arithmetic over values of unknown type: %', got;
    END IF;
    SELECT og_cypher('gaps', 'UNWIND [''b'', ''a'', ''c''] AS x RETURN min(x) AS lo, max(x) AS hi') INTO got;
    IF got <> '{"lo": "a", "hi": "c"}'::jsonb THEN RAISE EXCEPTION 'min/max of strings of unknown type: %', got; END IF;
    -- Stored nodes, directly and through WITH.
    SELECT og_cypher('gaps', 'MATCH (s:Shelf) WITH s RETURN sum(s.slots * s.width) AS s, max(s.width) AS w') INTO got;
    IF (got->>'s')::numeric <> 9 OR (got->>'w')::numeric <> 4 THEN
        RAISE EXCEPTION 'arithmetic on stored nodes through WITH: %', got;
    END IF;
    -- `+` with a string concatenates; with a list, appends.
    SELECT og_cypher('gaps', 'MATCH (m:Member {member_id: 1}) RETURN m.name + ''!'' AS a, m.name + 1 AS b,
                              collect(m.name) + [''x''] AS l') INTO got;
    IF got <> '{"a": "Ana!", "b": "Ana1", "l": ["Ana", "x"]}'::jsonb THEN RAISE EXCEPTION '+ on strings and lists: %', got; END IF;

    ---------------------------------------------------------------- 2. list slices
    SELECT og_cypher('gaps', 'MATCH (l:Loan)-[:BORROWED_BY]->(m:Member {member_id: 1}) WITH l ORDER BY l.loan_id
                              WITH collect(l.loan_id) AS ids
                              RETURN ids[0..1] AS a, ids[..1] AS b, ids[1..] AS c, ids[-1..] AS d,
                                     ids[5..9] AS e, ids[..-1] AS f, ids[null..1] AS g, ids[0] AS h') INTO got;
    IF got <> '{"a": [10], "b": [10], "c": [11], "d": [11], "e": [], "f": [10], "g": null, "h": 10}'::jsonb THEN
        RAISE EXCEPTION 'slices of a collected list: %', got;
    END IF;
    SELECT og_cypher('gaps', 'MATCH (m:Member) WITH m ORDER BY m.name RETURN collect(m.name)[0..2] AS two') INTO got;
    IF got <> '{"two": ["Ana", "Ben"]}'::jsonb THEN RAISE EXCEPTION 'collect()[0..2]: %', got; END IF;
    SELECT og_cypher('gaps', 'RETURN [1, 2, 3, 4][1..3] AS a, $xs[..2] AS b, $xs[-1..] AS c', '{"xs": [5, 6, 7]}') INTO got;
    IF got <> '{"a": [2, 3], "b": [5, 6], "c": [7]}'::jsonb THEN RAISE EXCEPTION 'slices of literals and parameters: %', got; END IF;
    SELECT og_cypher('gaps', 'MATCH (s:Shelf) WITH s ORDER BY s.code RETURN collect(s.code)[1..] AS rest') INTO got;
    IF got <> '{"rest": ["S2", "S3"]}'::jsonb THEN RAISE EXCEPTION 'slice over stored nodes: %', got; END IF;

    ---------------------------------------------------------------- 3. head() / last() / tail()
    SELECT og_cypher('gaps', 'MATCH (m:Member) WITH m ORDER BY m.name WITH collect(m.name) AS ns
                              RETURN head(ns) AS h, last(ns) AS l, tail(ns) AS t, head([]) AS e, tail([]) AS te') INTO got;
    IF got <> '{"h": "Ana", "l": "Cho", "t": ["Ben", "Cho"], "e": null, "te": []}'::jsonb THEN
        RAISE EXCEPTION 'head/last/tail: %', got;
    END IF;
    -- A list element compared with a typed value is read at that type.
    SELECT count(*) INTO n FROM og_cypher('gaps', 'MATCH (m:Member) WITH m ORDER BY m.name WITH collect(m.name) AS ns
                                                  WHERE head(ns) = ''Ana'' AND size(tail(ns)) = 2 RETURN ns');
    IF n <> 1 THEN RAISE EXCEPTION 'head() in a comparison: % rows', n; END IF;
    SELECT og_cypher('gaps', 'MATCH (s:Shelf) WITH s ORDER BY s.width DESC RETURN head(collect(s.code)) AS widest') INTO got;
    IF got <> '{"widest": "S3"}'::jsonb THEN RAISE EXCEPTION 'head() over stored nodes: %', got; END IF;

    ---------------------------------------------------------------- 4. label predicates
    SELECT og_cypher('gaps', 'MATCH (m:Member) WHERE m:Member OR m:Loan RETURN count(m) AS n') INTO got;
    IF (got->>'n')::int <> 3 THEN RAISE EXCEPTION 'n:A OR n:B: %', got; END IF;
    SELECT og_cypher('gaps', 'MATCH (l:Loan) WHERE NOT l:Member RETURN count(l) AS n') INTO got;
    IF (got->>'n')::int <> 5 THEN RAISE EXCEPTION 'NOT n:A: %', got; END IF;
    SELECT og_cypher('gaps', 'MATCH (l:Loan) WHERE l:Member RETURN count(l) AS n') INTO got;
    IF (got->>'n')::int <> 0 THEN RAISE EXCEPTION 'n:A on another type: %', got; END IF;
    SELECT og_cypher('gaps', 'MATCH (m:Member) WHERE m:Loan|Member AND NOT m:Member:Loan AND NOT m:Nowhere
                              RETURN count(m) AS n') INTO got;
    IF (got->>'n')::int <> 3 THEN RAISE EXCEPTION 'n:A|B, n:A:B, unknown label: %', got; END IF;
    SELECT og_cypher('gaps', 'MATCH (m:Member {member_id: 1}) RETURN m:Member AS is_m, m:Loan AS is_l') INTO got;
    IF got <> '{"is_m": true, "is_l": false}'::jsonb THEN RAISE EXCEPTION 'label predicate as a value: %', got; END IF;
    -- `|` inside a comprehension's WHERE is the projection, not another label.
    SELECT og_cypher('gaps', 'MATCH (m:Member) WITH m ORDER BY m.name WITH collect(m) AS ms
                              RETURN [x IN ms WHERE x:Member | x.name] AS names') INTO got;
    IF got <> '{"names": ["Ana", "Ben", "Cho"]}'::jsonb THEN RAISE EXCEPTION 'label predicate in a comprehension: %', got; END IF;
    -- An umbrella label beside a virtual type is read as MATCH (:_Entity:Member) reads it.
    SELECT og_cypher('gaps', 'MATCH (m:Member) WHERE m:_Entity RETURN count(m) AS n') INTO got;
    IF (got->>'n')::int <> 3 THEN RAISE EXCEPTION 'umbrella label on a virtual node: %', got; END IF;
    -- A node of no type yet under OPTIONAL MATCH: null, not false.
    SELECT count(*) INTO n FROM og_cypher('gaps', 'MATCH (m:Member) OPTIONAL MATCH (m)<-[:BORROWED_BY]-(l:Loan {loan_id: 10})
                                                  WITH m, l WHERE (l:Loan) IS NULL RETURN m');
    IF n <> 2 THEN RAISE EXCEPTION 'label predicate on an OPTIONAL null: % rows', n; END IF;
    -- Stored nodes: a label is carried by its subtypes.
    SELECT og_cypher('gaps', 'MATCH (a:Asset) WHERE a:Shelf RETURN count(a) AS shelves') INTO got;
    IF (got->>'shelves')::int <> 3 THEN RAISE EXCEPTION 'subtype label predicate: %', got; END IF;
    SELECT og_cypher('gaps', 'MATCH (a:Asset) WHERE a:Asset AND NOT a:Shelf RETURN collect(a.code) AS c') INTO got;
    IF got <> '{"c": ["A1"]}'::jsonb THEN RAISE EXCEPTION 'supertype label predicate: %', got; END IF;
    SELECT og_cypher('gaps', 'MATCH (n:_Entity) WHERE n:Asset RETURN count(n) AS n') INTO got;
    IF (got->>'n')::int <> 4 THEN RAISE EXCEPTION 'label predicate under an umbrella: %', got; END IF;
    -- Over a virtual node the predicate is a constant: nothing of it is left to run.
    SELECT og_cypher_sql('gaps', 'MATCH (m:Member) WHERE m:Member RETURN m.name') INTO sql;
    IF sql LIKE '%og_is_subtype%' THEN RAISE EXCEPTION 'label predicate evaluated per row: %', sql; END IF;

    ---------------------------------------------------------------- 5. virtual hops with an unlabelled end
    SELECT og_cypher('gaps', 'MATCH (l:Loan)-[:BORROWED_BY]->(m) RETURN count(m) AS n') INTO got;
    IF (got->>'n')::int <> 5 THEN RAISE EXCEPTION 'unlabelled far end: %', got; END IF;
    SELECT og_cypher('gaps', 'MATCH (l:Loan)-[:BORROWED_BY]-(m) RETURN count(DISTINCT m.name) AS n') INTO got;
    IF (got->>'n')::int <> 3 THEN RAISE EXCEPTION 'undirected, unlabelled far end: %', got; END IF;
    SELECT og_cypher('gaps', 'MATCH (l)-[:BORROWED_BY]-(m:Member {name: ''Ana''}) RETURN sum(l.days) AS d') INTO got;
    IF (got->>'d')::int <> 5 THEN RAISE EXCEPTION 'undirected, unlabelled start: %', got; END IF;
    -- A node carried through WITH anchors the hop again, with or without its label.
    SELECT og_cypher('gaps', 'MATCH (m:Member) WITH m MATCH (m)<-[:BORROWED_BY]-(l:Loan) RETURN count(l) AS n') INTO got;
    IF (got->>'n')::int <> 5 THEN RAISE EXCEPTION 'carried node, no label: %', got; END IF;
    SELECT og_cypher('gaps', 'MATCH (m:Member) WITH m ORDER BY m.name LIMIT 1
                              MATCH (m:Member)<-[:BORROWED_BY]-(l) RETURN m.name AS name, count(l) AS n') INTO got;
    IF got <> '{"name": "Ana", "n": 2}'::jsonb THEN RAISE EXCEPTION 'carried node, labelled, after LIMIT: %', got; END IF;
    SELECT og_cypher('gaps', 'MATCH (l:Loan) WITH l WHERE l.days >= 3 MATCH (l)-[:BORROWED_BY]->(m) RETURN collect(m.name) AS c') INTO got;
    IF jsonb_array_length(got->'c') <> 2 THEN RAISE EXCEPTION 'carried node, unlabelled far end: %', got; END IF;
    SELECT count(*) INTO n FROM og_cypher('gaps', 'MATCH (m:Member) WITH m
                                                  OPTIONAL MATCH (m)<-[:BORROWED_BY]-(l:Loan {returned: false}) RETURN m.name, l.loan_id');
    IF n <> 3 THEN RAISE EXCEPTION 'carried node under OPTIONAL MATCH: % rows', n; END IF;
    -- Both ends unlabelled and no direction: still refused, saying how to fix it.
    failed := false;
    BEGIN
        PERFORM og_cypher('gaps', 'MATCH (a)-[:BORROWED_BY]-(b) RETURN count(*)');
    EXCEPTION WHEN others THEN
        failed := SQLERRM LIKE '%Label one end, or give the hop a direction%';
    END;
    IF NOT failed THEN RAISE EXCEPTION 'an undirected hop with no label anywhere was not refused helpfully'; END IF;

    ---------------------------------------------------------------- 5b. a source key named `id`
    SELECT og_cypher('gaps', 'MATCH (b:Branch {id: 1}) RETURN b.city AS city, b.id AS id') INTO got;
    IF got <> '{"city": "North", "id": 1}'::jsonb THEN RAISE EXCEPTION 'n.id on a virtual type: %', got; END IF;
    SELECT og_cypher('gaps', 'MATCH (d:Desk)-[:AT_BRANCH]->(b:Branch) RETURN b.city AS city, count(d) AS n ORDER BY city') INTO got;
    IF got <> '{"city": "North", "n": 2}'::jsonb THEN RAISE EXCEPTION 'virtual hop onto an id key: %', got; END IF;
    SELECT og_cypher('gaps', 'MATCH (b:Branch) WITH b ORDER BY b.id DESC LIMIT 1 MATCH (b)<-[:AT_BRANCH]-(d) RETURN collect(d.id) AS c') INTO got;
    IF got <> '{"c": [9]}'::jsonb THEN RAISE EXCEPTION 'carried node with an id key: %', got; END IF;
    -- id() is still the node's identifier, and so is _id.
    SELECT og_cypher('gaps', 'MATCH (b:Branch {id: 2}) RETURN id(b) = b._id AS same, id(b) <> b.id AS differs') INTO got;
    IF got <> '{"same": true, "differs": true}'::jsonb THEN RAISE EXCEPTION 'id() beside an id property: %', got; END IF;

    ---------------------------------------------------------------- 6. booleans against 0 / 1
    SELECT og_cypher('gaps', 'MATCH (m:Member) WHERE m.active = 1 RETURN collect(m.name) AS c') INTO got;
    IF got <> '{"c": ["Ana"]}'::jsonb THEN RAISE EXCEPTION 'boolean = 1: %', got; END IF;
    SELECT og_cypher('gaps', 'MATCH (m:Member) WHERE m.active = 0 RETURN collect(m.name) AS c') INTO got;
    IF got <> '{"c": ["Ben"]}'::jsonb THEN RAISE EXCEPTION 'boolean = 0: %', got; END IF;
    SELECT og_cypher('gaps', 'MATCH (m:Member) WHERE m.active <> 1 RETURN count(m) AS n') INTO got;
    IF (got->>'n')::int <> 1 THEN RAISE EXCEPTION 'boolean <> 1 (null is neither): %', got; END IF;
    SELECT og_cypher('gaps', 'MATCH (m:Member) WHERE m.active = 2 OR 1 = m.active RETURN count(m) AS n') INTO got;
    IF (got->>'n')::int <> 1 THEN RAISE EXCEPTION 'boolean = 2 never holds, 1 = boolean does: %', got; END IF;
    SELECT og_cypher('gaps', 'MATCH (m:Member) WHERE m.active IN [1] RETURN count(m) AS a') INTO got;
    IF (got->>'a')::int <> 1 THEN RAISE EXCEPTION 'boolean IN [1]: %', got; END IF;
    SELECT og_cypher('gaps', 'MATCH (m:Member) WHERE m.active IN [0, 1] RETURN count(m) AS a') INTO got;
    IF (got->>'a')::int <> 2 THEN RAISE EXCEPTION 'boolean IN [0, 1]: %', got; END IF;
    -- The other way round: a 0/1 column against true.
    SELECT og_cypher('gaps', 'MATCH (m:Member) WHERE m.flag = true RETURN collect(m.name) AS c') INTO got;
    IF got <> '{"c": ["Ana"]}'::jsonb THEN RAISE EXCEPTION '0/1 column = true: %', got; END IF;
    -- Carried through WITH, and on stored nodes.
    SELECT og_cypher('gaps', 'MATCH (m:Member) WITH m.active AS a WHERE a = 1 RETURN count(*) AS n') INTO got;
    IF (got->>'n')::int <> 1 THEN RAISE EXCEPTION 'carried boolean = 1: %', got; END IF;
    SELECT og_cypher('gaps', 'MATCH (s:Shelf) WHERE s.open = 1 RETURN count(s) AS n') INTO got;
    IF (got->>'n')::int <> 2 THEN RAISE EXCEPTION 'stored boolean = 1: %', got; END IF;
    -- `= 1` stays a comparison on the column, which the source is sent.
    SELECT og_cypher_sql('gaps', 'MATCH (m:Member) WHERE m.active = 1 RETURN m.name') INTO sql;
    IF sql NOT LIKE '%p_active = true%' THEN RAISE EXCEPTION 'boolean = 1 compiled as %', sql; END IF;
END
$$;
