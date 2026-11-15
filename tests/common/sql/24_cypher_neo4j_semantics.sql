-- Neo4j semantics the engine used to differ on, most of them without an error.
--
--   1. `id` is an ordinary property, as in Neo4j: `CREATE (n:L {id: 'singleton'})`
--      then `MATCH (n:L {id: $id})` / `n.id`. Stored types read `n.id` as the
--      internal identifier (a number), and `{id: 'singleton'}` failed with
--      "invalid input syntax for type bigint". id(n), elementId(n), n._id stay
--      the identifier.
--   2. An unlabelled MATCH — `(n)`, `(a)-[r]->(b)` — read every graph in the
--      database; `MATCH (n) DETACH DELETE n` emptied all of them.
--   3. `UNWIND $rows AS item MERGE (n:L {k: item.k})` never matched, so every
--      run created duplicates (or broke a unique constraint).
--   4. `WITH x AS alias` (and any expression) before a write was refused; a
--      reading clause after a write was skipped without a word.
--   5. size() of a missing property / null — "function jsonb_array_length(text)
--      does not exist"; size() of a string.
--   6. `WITH $row AS r RETURN r.name` was null; `$row.name` refused;
--      coalesce(n.list, $list) returned the list as a JSON string; nodes(p)
--      gave nulls.
-- No expected errors: any error fails the file.
\set ON_ERROR_STOP on

SELECT og_create_graph('sem');
SELECT og_create_graph('sem_other');

DO $$
DECLARE got jsonb; n int; failed bool;
BEGIN
    ------------------------------------------------------------- 1. id is a property
    PERFORM og_cypher('sem', $q$CREATE (n:Lamp {id: 'singleton', watts: 40})$q$);
    SELECT og_cypher('sem', $q$MATCH (n:Lamp) RETURN n.id AS id$q$) INTO got;
    IF got <> '{"id": "singleton"}' THEN RAISE EXCEPTION 'n.id on a stored type: %', got; END IF;
    SELECT og_cypher('sem', $q$MATCH (n:Lamp {id: $id}) RETURN n.watts AS w$q$, '{"id": "singleton"}') INTO got;
    IF got <> '{"w": 40}' THEN RAISE EXCEPTION '{id: $id} on a stored type: %', got; END IF;
    SELECT og_cypher('sem', $q$MATCH (n:Lamp) WHERE n.id = 'singleton' RETURN count(n) AS c$q$) INTO got;
    IF got <> '{"c": 1}' THEN RAISE EXCEPTION 'WHERE n.id = …: %', got; END IF;
    -- The identifier is still there, three ways, and is not the property.
    SELECT og_cypher('sem', $q$MATCH (n:Lamp) RETURN id(n) = n._id AS same, elementId(n) = toString(id(n)) AS eid,
                                id(n) > 0 AS positive$q$) INTO got;
    IF got <> '{"same": true, "eid": true, "positive": true}' THEN RAISE EXCEPTION 'id() / _id: %', got; END IF;
    -- A node without an `id` property has a null n.id, as in Neo4j.
    PERFORM og_cypher('sem', $q$CREATE (:Lamp {watts: 60})$q$);
    SELECT og_cypher('sem', $q$MATCH (n:Lamp {watts: 60}) RETURN n.id AS id$q$) INTO got;
    IF got <> '{"id": null}' THEN RAISE EXCEPTION 'n.id where none was set: %', got; END IF;
    -- MERGE on it finds the node rather than failing or duplicating.
    PERFORM og_cypher('sem', $q$MERGE (n:Lamp {id: 'singleton'}) ON MATCH SET n.seen = true$q$);
    SELECT og_cypher('sem', $q$MATCH (n:Lamp {id: 'singleton'}) RETURN count(n) AS c, collect(n.seen) AS s$q$) INTO got;
    IF got <> '{"c": 1, "s": [true]}' THEN RAISE EXCEPTION 'MERGE on {id: …}: %', got; END IF;
    -- A number in it is a number, an unlabelled pattern reads it too.
    PERFORM og_cypher('sem', $q$CREATE (:Socket {id: 7})$q$);
    SELECT og_cypher('sem', $q$MATCH (n {id: 7}) RETURN labels(n) AS l, n.id + 1 AS next$q$) INTO got;
    IF got <> '{"l": ["Socket"], "next": 8}' THEN RAISE EXCEPTION 'numeric id property: %', got; END IF;

    ------------------------------------------------------------- 2. unlabelled MATCH stays in its graph
    PERFORM og_cypher('sem_other', $q$CREATE (:Far {name: 'x'})-[:LINK]->(:Far {name: 'y'})$q$);
    SELECT og_cypher('sem', $q$MATCH (n) RETURN count(n) AS c$q$) INTO got;
    IF got <> '{"c": 3}' THEN RAISE EXCEPTION 'MATCH (n) saw another graph: %', got; END IF;
    SELECT og_cypher('sem', $q$MATCH (a)-[r]->(b) RETURN count(r) AS c$q$) INTO got;
    IF got <> '{"c": 0}' THEN RAISE EXCEPTION '(a)-[r]->(b) saw another graph: %', got; END IF;
    SELECT og_cypher('sem', $q$MATCH (n) WHERE n.name = 'x' RETURN count(n) AS c$q$) INTO got;
    IF got <> '{"c": 0}' THEN RAISE EXCEPTION 'unlabelled WHERE saw another graph: %', got; END IF;
    SELECT og_cypher('sem_other', $q$MATCH (a)-[r]->(b) RETURN a.name AS a, b.name AS b$q$) INTO got;
    IF got <> '{"a": "x", "b": "y"}' THEN RAISE EXCEPTION 'the other graph lost its own: %', got; END IF;
    -- The one that matters most: deleting everything deletes this graph's everything.
    PERFORM og_cypher('sem_other', $q$CREATE (:Scratch)$q$);
    PERFORM og_cypher('sem_other', $q$MATCH (n:Scratch) DETACH DELETE n$q$);
    PERFORM og_create_graph('sem_tmp');
    PERFORM og_cypher('sem_tmp', $q$CREATE (:Tmp)-[:T]->(:Tmp)$q$);
    PERFORM og_cypher('sem_tmp', $q$MATCH (n) DETACH DELETE n$q$);
    SELECT og_cypher('sem_other', $q$MATCH (n) RETURN count(n) AS c$q$) INTO got;
    IF got <> '{"c": 2}' THEN RAISE EXCEPTION 'MATCH (n) DETACH DELETE n reached another graph: %', got; END IF;
    SELECT og_cypher('sem', $q$MATCH (n) RETURN count(n) AS c$q$) INTO got;
    IF got <> '{"c": 3}' THEN RAISE EXCEPTION 'MATCH (n) DETACH DELETE n reached this graph: %', got; END IF;

    ------------------------------------------------------------- 3. UNWIND … MERGE matches
    PERFORM og_cypher('sem', $q$CREATE (:Part {product_id: 'p1', qty: 1})$q$);
    PERFORM og_cypher('sem', $q$UNWIND $rows AS item MERGE (n:Part {product_id: item.node_id})
                              ON CREATE SET n.qty = item.qty ON MATCH SET n.qty = n.qty + item.qty$q$,
                      '{"rows": [{"node_id": "p1", "qty": 5}, {"node_id": "p2", "qty": 6}]}');
    PERFORM og_cypher('sem', $q$UNWIND $rows AS item MERGE (n:Part {product_id: item.node_id})$q$,
                      '{"rows": [{"node_id": "p1"}, {"node_id": "p2"}]}');
    SELECT jsonb_agg(r ORDER BY r->>'p') FROM og_cypher('sem', $q$MATCH (n:Part) RETURN n.product_id AS p, n.qty AS q$q$) r INTO got;
    IF got <> '[{"p": "p1", "q": 6}, {"p": "p2", "q": 6}]' THEN RAISE EXCEPTION 'UNWIND MERGE: %', got; END IF;
    -- Under a unique constraint it must not fail either.
    PERFORM og_cypher('sem', $q$CREATE CONSTRAINT part_pid IF NOT EXISTS FOR (n:Part) REQUIRE n.product_id IS UNIQUE$q$);
    PERFORM og_cypher('sem', $q$UNWIND $ids AS k MERGE (n:Part {product_id: k}) RETURN count(n) AS c$q$, '{"ids": ["p1", "p2", "p3"]}');
    SELECT og_cypher('sem', $q$MATCH (n:Part) RETURN count(n) AS c$q$) INTO got;
    IF got <> '{"c": 3}' THEN RAISE EXCEPTION 'UNWIND MERGE under a constraint: %', got; END IF;
    -- A relationship merged between rows' nodes, once.
    PERFORM og_cypher('sem', $q$UNWIND $pairs AS pr MATCH (a:Part {product_id: pr.a}), (b:Part {product_id: pr.b})
                              MERGE (a)-[:FITS {via: pr.via}]->(b)$q$,
                      '{"pairs": [{"a": "p1", "b": "p2", "via": "x"}, {"a": "p1", "b": "p2", "via": "x"}]}');
    SELECT og_cypher('sem', $q$MATCH (:Part)-[f:FITS]->(:Part) RETURN count(f) AS c$q$) INTO got;
    IF got <> '{"c": 1}' THEN RAISE EXCEPTION 'UNWIND MERGE of a relationship: %', got; END IF;
    -- A pattern with no variable is looked up too.
    PERFORM og_cypher('sem', $q$MERGE (:Tag {name: 'only'})$q$);
    PERFORM og_cypher('sem', $q$MERGE (:Tag {name: 'only'})$q$);
    SELECT og_cypher('sem', $q$MATCH (t:Tag) RETURN count(t) AS c$q$) INTO got;
    IF got <> '{"c": 1}' THEN RAISE EXCEPTION 'MERGE without a variable: %', got; END IF;
    -- A null key is refused, as Neo4j refuses it (it would create a node per run).
    failed := false;
    BEGIN
        PERFORM og_cypher('sem', $q$UNWIND $rows AS item MERGE (n:Part {product_id: item.missing})$q$, '{"rows": [{"x": 1}]}');
    EXCEPTION WHEN OTHERS THEN
        failed := SQLERRM LIKE '%null property value for ''product_id''%';
    END;
    IF NOT failed THEN RAISE EXCEPTION 'MERGE on a null key was not refused'; END IF;

    ------------------------------------------------------------- 4. WITH before (and after) a write
    SELECT jsonb_agg(r ORDER BY r->>'p') FROM og_cypher('sem', $q$MATCH (n:Part) WITH n AS m SET m.checked = true RETURN m.product_id AS p$q$) r INTO got;
    IF got <> '[{"p": "p1"}, {"p": "p2"}, {"p": "p3"}]' THEN RAISE EXCEPTION 'WITH n AS m SET: %', got; END IF;
    SELECT og_cypher('sem', $q$MATCH (n:Part) WHERE n.checked RETURN count(n) AS c$q$) INTO got;
    IF got <> '{"c": 3}' THEN RAISE EXCEPTION 'WITH n AS m SET did not write: %', got; END IF;
    -- Expressions and aggregates before a write.
    PERFORM og_cypher('sem', $q$MATCH (n:Part {product_id: 'p1'}) WITH n, n.qty * 2 AS twice SET n.twice = twice$q$);
    PERFORM og_cypher('sem', $q$MATCH (n:Part) WITH count(n) AS total MERGE (s:Summary {name: 'parts'}) SET s.total = total$q$);
    SELECT og_cypher('sem', $q$MATCH (n:Part {product_id: 'p1'}), (s:Summary) RETURN n.twice AS t, s.total AS total$q$) INTO got;
    IF got <> '{"t": 12, "total": 3}' THEN RAISE EXCEPTION 'computed WITH before a write: %', got; END IF;
    -- A reading clause after a write is run, not skipped.
    PERFORM og_cypher('sem', $q$MERGE (a:Kit {name: 'k1'}) WITH a MATCH (p:Part {product_id: 'p2'}) MERGE (a)-[:HOLDS]->(p)$q$);
    SELECT og_cypher('sem', $q$MATCH (k:Kit)-[:HOLDS]->(p) RETURN k.name AS k, p.product_id AS p, labels(p) AS l$q$) INTO got;
    IF got <> '{"k": "k1", "p": "p2", "l": ["Part"]}' THEN RAISE EXCEPTION 'MATCH after MERGE: %', got; END IF;
    -- A write runs once per row, even when the row binds no variable.
    PERFORM og_cypher('sem', $q$MATCH (:Part) CREATE (:Receipt)$q$);
    SELECT og_cypher('sem', $q$MATCH (r:Receipt) RETURN count(r) AS c$q$) INTO got;
    IF got <> '{"c": 3}' THEN RAISE EXCEPTION 'CREATE per matched row: %', got; END IF;
    -- RETURN after a write has ORDER BY, LIMIT and aggregates.
    SELECT jsonb_agg(r) FROM og_cypher('sem', $q$MATCH (n:Part) SET n.seen = 1 RETURN n.product_id AS p ORDER BY p DESC LIMIT 2$q$) r INTO got;
    IF got <> '[{"p": "p3"}, {"p": "p2"}]' THEN RAISE EXCEPTION 'RETURN … ORDER BY LIMIT after a write: %', got; END IF;
    SELECT og_cypher('sem', $q$MATCH (n:Part) SET n.seen = 2 RETURN count(n) AS c, sum(n.seen) AS s$q$) INTO got;
    IF got <> '{"c": 3, "s": 6}' THEN RAISE EXCEPTION 'aggregates after a write: %', got; END IF;
    -- A rename still happens on the matched rows (and only once).
    PERFORM og_cypher('sem', $q$CREATE (:OldName {k: 1}), (:OldName {k: 2})$q$);
    SELECT og_cypher('sem', $q$MATCH (n:OldName) REMOVE n:OldName SET n:NewName RETURN count(n) AS c$q$) INTO got;
    IF got <> '{"c": 2}' THEN RAISE EXCEPTION 'rename by REMOVE/SET: %', got; END IF;

    ------------------------------------------------------------- 5. size()
    SELECT og_cypher('sem', $q$MATCH (n:Lamp {id: 'singleton'}) RETURN size(n.missing) AS a, size(null) AS b,
                                size(n.id) AS c, size('héllo') AS d, size([1, 2, 3]) AS e, size($l) AS f, size($s) AS g$q$,
                      '{"l": [1, 2], "s": "abc"}') INTO got;
    IF got <> '{"a": null, "b": null, "c": 9, "d": 5, "e": 3, "f": 2, "g": 3}' THEN RAISE EXCEPTION 'size(): %', got; END IF;
    PERFORM og_cypher('sem', $q$MATCH (n:Lamp {id: 'singleton'}) SET n.tags = ['a', 'b']$q$);
END $$;

DO $$
DECLARE got jsonb; failed bool;
BEGIN
    SELECT og_cypher('sem', $q$MATCH (n:Lamp {id: 'singleton'}) WITH n.tags AS t RETURN size(t) AS s$q$) INTO got;
    IF got <> '{"s": 2}' THEN RAISE EXCEPTION 'size() of a list carried through WITH: %', got; END IF;
    failed := false;
    BEGIN
        PERFORM og_cypher('sem', $q$RETURN size({a: 1}) AS s$q$);
    EXCEPTION WHEN OTHERS THEN failed := SQLERRM LIKE '%a map has no size%';
    END;
    IF NOT failed THEN RAISE EXCEPTION 'size() of a map literal was not refused'; END IF;

    ------------------------------------------------------------- 6. parameters, coalesce, paths
    SELECT og_cypher('sem', $q$WITH $row AS r RETURN r.name AS n, r.qty + 1 AS q$q$, '{"row": {"name": "zz", "qty": 2}}') INTO got;
    IF got <> '{"n": "zz", "q": 3}' THEN RAISE EXCEPTION 'WITH $row AS r: %', got; END IF;
    SELECT og_cypher('sem', $q$RETURN $row.name AS n, $row.inner.k AS k, $n AS num, $l AS list$q$,
                      '{"row": {"name": "zz", "inner": {"k": true}}, "n": 5, "l": [1, "a"]}') INTO got;
    IF got <> '{"n": "zz", "k": true, "num": 5, "list": [1, "a"]}' THEN RAISE EXCEPTION '$row.name / typed parameters: %', got; END IF;
    SELECT og_cypher('sem', $q$UNWIND $rows AS r WITH r WHERE r.v > 1 RETURN collect(r.k) AS ks$q$,
                      '{"rows": [{"k": "a", "v": 1}, {"k": "b", "v": 2}]}') INTO got;
    IF got <> '{"ks": ["b"]}' THEN RAISE EXCEPTION 'UNWIND of maps: %', got; END IF;
    -- coalesce keeps a list a list.
    SELECT og_cypher('sem', $q$MATCH (n:Lamp {watts: 60}) RETURN coalesce(n.tags, $l) AS a, coalesce(n.tags, []) AS b,
                                coalesce(n.nothing, n.watts) AS c, coalesce(n.id, 'none') AS d$q$, '{"l": ["x", "y"]}') INTO got;
    IF got <> '{"a": ["x", "y"], "b": [], "c": 60, "d": "none"}' THEN RAISE EXCEPTION 'coalesce: %', got; END IF;
    SELECT og_cypher('sem', $q$MATCH (n:Lamp {id: 'singleton'}) RETURN coalesce(n.tags, $l) AS a$q$, '{"l": ["x"]}') INTO got;
    IF got <> '{"a": ["a", "b"]}' THEN RAISE EXCEPTION 'coalesce with a value present: %', got; END IF;
    -- A property of any value: coalesce(…).name over OPTIONAL MATCH results.
    PERFORM og_cypher('sem', $q$CREATE (:Story {name: 's1'})-[:IN]->(:Ctx {name: 'c1'}), (:Story {name: 's2'})$q$);
    SELECT jsonb_agg(r ORDER BY r->>'s') FROM og_cypher('sem', $q$MATCH (s:Story) OPTIONAL MATCH (s)-[:IN]->(c:Ctx)
                                OPTIONAL MATCH (s)-[:IN]->(d:Nowhere) RETURN s.name AS s, coalesce(d, c).name AS ctx$q$) r INTO got;
    IF got <> '[{"s": "s1", "ctx": "c1"}, {"s": "s2", "ctx": null}]' THEN RAISE EXCEPTION 'coalesce(x, y).name: %', got; END IF;
    -- An OPTIONAL MATCH that opens the query and matches nothing is one row of nulls.
    SELECT jsonb_agg(r) FROM og_cypher('sem', $q$OPTIONAL MATCH (n:Story {name: 'nope'}) OPTIONAL MATCH (n)-[:IN]->(m) RETURN n, m$q$) r INTO got;
    IF got <> '[{"n": null, "m": null}]' THEN RAISE EXCEPTION 'leading OPTIONAL MATCH with no match: %', got; END IF;
    -- nodes(p), relationships(p), length(p), on fixed and variable-length paths.
    PERFORM og_cypher('sem', $q$CREATE (:Hop {i: 1})-[:NEXT {w: 1}]->(:Hop {i: 2})-[:NEXT {w: 2}]->(:Hop {i: 3})$q$);
    SELECT og_cypher('sem', $q$MATCH p = (a:Hop {i: 1})-[:NEXT]->(b)-[:NEXT]->(c)
                                RETURN [x IN nodes(p) | x.i] AS ns, [r IN relationships(p) | r.w] AS ws, length(p) AS l$q$) INTO got;
    IF got <> '{"ns": [1, 2, 3], "ws": [1, 2], "l": 2}' THEN RAISE EXCEPTION 'nodes(p) on a fixed path: %', got; END IF;
    SELECT jsonb_agg(r ORDER BY (r->>'l')::int) FROM og_cypher('sem', $q$MATCH p = (a:Hop {i: 1})-[:NEXT*1..3]->(b)
                                RETURN [x IN nodes(p) | x.i] AS ns, length(p) AS l, size(relationships(p)) AS r$q$) r INTO got;
    IF got <> '[{"ns": [1, 2], "l": 1, "r": 1}, {"ns": [1, 2, 3], "l": 2, "r": 2}]' THEN RAISE EXCEPTION 'nodes(p) on a variable-length path: %', got; END IF;
    SELECT og_cypher('sem', $q$MATCH p = (c:Hop {i: 3})<-[:NEXT*2]-(a) RETURN [x IN nodes(p) | x.i] AS ns$q$) INTO got;
    IF got <> '{"ns": [3, 2, 1]}' THEN RAISE EXCEPTION 'nodes(p) walked against the arrows: %', got; END IF;
    SELECT og_cypher('sem', $q$MATCH (a:Hop {i: 1})-[rs:NEXT*2]->(c) RETURN [r IN rs | r.w] AS ws, size(rs) AS n$q$) INTO got;
    IF got <> '{"ws": [1, 2], "n": 2}' THEN RAISE EXCEPTION 'a variable-length relationship variable: %', got; END IF;
    SELECT og_cypher('sem', $q$MATCH p = (a:Hop {i: 1})-[:NEXT]->(b) RETURN p$q$) INTO got;
    IF got->'p'->>'_path' <> 'true' OR jsonb_array_length(got->'p'->'nodes') <> 2 THEN RAISE EXCEPTION 'a path value: %', got; END IF;
    -- CASE over a list and a parameter, a string literal IN a list.
    SELECT jsonb_agg(r) FROM og_cypher('sem', $q$UNWIND (CASE WHEN $x IS NULL THEN [null] ELSE $x END) AS v RETURN v$q$, '{"x": null}') r INTO got;
    IF got <> '[{"v": null}]' THEN RAISE EXCEPTION 'CASE of a list and a parameter: %', got; END IF;
    SELECT og_cypher('sem', $q$MATCH (n:Hop) WHERE 'Hop' IN labels(n) AND $i IN [1, 2] RETURN count(n) AS c$q$, '{"i": 2}') INTO got;
    IF got <> '{"c": 3}' THEN RAISE EXCEPTION '''X'' IN labels(n), $i IN [1, 2]: %', got; END IF;
    -- An unwound value against an untyped property of an unlabelled node.
    SELECT og_cypher('sem', $q$UNWIND $ids AS x MATCH (n {name: x}) RETURN count(n) AS c$q$, '{"ids": ["s1", "c1"]}') INTO got;
    IF got <> '{"c": 2}' THEN RAISE EXCEPTION 'UNWIND x MATCH (n {k: x}): %', got; END IF;
    -- A write path evaluator error is an error, not a null.
    failed := false;
    BEGIN
        PERFORM og_cypher('sem', $q$MATCH (n:Hop {i: 1}) SET n.x = noSuchFunction(1)$q$);
    EXCEPTION WHEN OTHERS THEN failed := SQLERRM LIKE '%nosuchfunction%';
    END;
    IF NOT failed THEN RAISE EXCEPTION 'an unknown function in SET was not refused'; END IF;
END $$;
