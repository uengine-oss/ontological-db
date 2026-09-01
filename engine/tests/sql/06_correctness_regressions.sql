-- Regressions for five defects that returned a wrong answer without an error.
--
-- These assert rather than print. run.sh compares the number of ERROR lines
-- against the number of expected-error markers in the file, so a RAISE
-- EXCEPTION here fails the suite — which is what makes this file a gate rather
-- than a report. Exactly one error is expected, at the very end.
-- Every check below fails on the code as it stood at 7d60c82.
\set ON_ERROR_STOP on

SELECT og_create_graph('reg');
SELECT og_create_type('reg', 'P', 'entity');
SELECT og_add_property('reg', 'P', 'name', 'string');
SELECT og_create_type('reg', 'Q', 'entity');
SELECT og_add_property('reg', 'Q', 'name', 'string');
SELECT og_create_type('reg', 'L', 'relation');
SELECT og_add_role('reg', 'L', 'src', 'P', 0);
SELECT og_add_role('reg', 'L', 'dst', 'P', 1);

-- Names arrive interleaved on purpose: a, b, a, b. Sorted input would hide the
-- DISTINCT defect, because consecutive-only deduplication is correct there.
SELECT og_cypher('reg', 'CREATE (:P {name:"a"})');
SELECT og_cypher('reg', 'CREATE (:P {name:"b"})');
SELECT og_cypher('reg', 'CREATE (:P {name:"a"})');
SELECT og_cypher('reg', 'CREATE (:P {name:"b"})');
SELECT og_cypher('reg', 'CREATE (:Q {name:"q1"})');
SELECT og_cypher('reg', 'CREATE (:Q {name:"q2"})');

\echo '--- UNION returns every branch ---'
-- The parser has always built Query.union; nothing read it, so this returned
-- the four P rows and no error.
DO $$
DECLARE n int;
BEGIN
    SELECT count(*) INTO n FROM og_cypher('reg',
        'MATCH (p:P) RETURN p.name AS name UNION ALL MATCH (q:Q) RETURN q.name AS name');
    IF n <> 6 THEN
        RAISE EXCEPTION 'UNION ALL: expected 6 rows, got %', n;
    END IF;

    SELECT count(*) INTO n FROM og_cypher('reg',
        'MATCH (p:P) RETURN p.name AS name UNION MATCH (q:Q) RETURN q.name AS name');
    IF n <> 4 THEN
        RAISE EXCEPTION 'UNION: expected 4 distinct rows (a,b,q1,q2), got %', n;
    END IF;
END
$$;

\echo '--- count(DISTINCT) counts values, not runs ---'
-- This has to be a *write* query. A read compiles to SQL and PostgreSQL's own
-- count(DISTINCT) was always right; the defect is in the write path, which
-- walks bindings row by row and folds the aggregate itself over what it saw.
-- The rows arrive as a, b, a, b — so consecutive-only deduplication removes
-- nothing and the count comes back 4.
DO $$
DECLARE got jsonb;
BEGIN
    SELECT og_cypher('reg',
        'MATCH (p:P) SET p.name = p.name RETURN count(DISTINCT p.name) AS n') INTO got;
    IF (got->>'n')::int <> 2 THEN
        RAISE EXCEPTION 'count(DISTINCT) on the write path: expected 2, got % (Vec::dedup only removes runs)',
            got->>'n';
    END IF;

    SELECT og_cypher('reg',
        'MATCH (p:P) SET p.name = p.name RETURN collect(DISTINCT p.name) AS c') INTO got;
    IF jsonb_array_length(got->'c') <> 2 THEN
        RAISE EXCEPTION 'collect(DISTINCT) on the write path: expected 2 elements, got %',
            jsonb_array_length(got->'c');
    END IF;
END
$$;

\echo '--- a compiled plan does not outlive its schema ---'
-- The cache was keyed on (graph, query) alone. Adding a property bumps the
-- schema version, which drops the generated views the cached SQL names.
DO $$
DECLARE n int;
BEGIN
    SELECT count(*) INTO n FROM og_cypher('reg', 'MATCH (p:P) RETURN p.name AS name');
    IF n <> 4 THEN
        RAISE EXCEPTION 'setup: expected 4 P rows, got %', n;
    END IF;

    PERFORM og_add_property('reg', 'P', 'tag', 'string');

    SELECT count(*) INTO n FROM og_cypher('reg', 'MATCH (p:P) RETURN p.name AS name');
    IF n <> 4 THEN
        RAISE EXCEPTION 'after a schema change the same query returned % rows', n;
    END IF;
END
$$;

\echo '--- *min..max with min > 1 does not lose nodes ---'
-- s reaches t1 in one hop and again in four (s->x->y->z->t1). og_reach visits
-- each node once and emits it at its shortest distance, so t1 is marked visited
-- at depth 1, never emitted for min=2, and never reconsidered. og_vlp
-- enumerates trails and finds the four-hop walk. The rewrite must not be chosen
-- where the two disagree.
SELECT og_cypher('reg', 'CREATE (:P {name:"s"})');
SELECT og_cypher('reg', 'CREATE (:P {name:"t1"})');
SELECT og_cypher('reg', 'CREATE (:P {name:"x"})');
SELECT og_cypher('reg', 'CREATE (:P {name:"y"})');
SELECT og_cypher('reg', 'CREATE (:P {name:"z"})');
SELECT og_cypher('reg', 'MATCH (a:P {name:"s"}),(b:P {name:"t1"}) CREATE (a)-[:L]->(b)');
SELECT og_cypher('reg', 'MATCH (a:P {name:"s"}),(b:P {name:"x"})  CREATE (a)-[:L]->(b)');
SELECT og_cypher('reg', 'MATCH (a:P {name:"x"}),(b:P {name:"y"})  CREATE (a)-[:L]->(b)');
SELECT og_cypher('reg', 'MATCH (a:P {name:"y"}),(b:P {name:"z"})  CREATE (a)-[:L]->(b)');
SELECT og_cypher('reg', 'MATCH (a:P {name:"z"}),(b:P {name:"t1"}) CREATE (a)-[:L]->(b)');

DO $$
DECLARE found boolean;
BEGIN
    SELECT bool_or(r->>'n' = 't1') INTO found
      FROM og_cypher('reg',
          'MATCH (s:P {name:"s"})-[*2..4]->(m) RETURN DISTINCT m.name AS n') AS r;
    IF NOT COALESCE(found, false) THEN
        RAISE EXCEPTION
            '*2..4 lost t1: reachable in four hops but marked visited at one';
    END IF;
END
$$;

\echo '--- WITH ... LIMIT before a write is honoured ---'
-- take_while(Match|Unwind) stopped at WITH, and the write loop ignored it, so
-- the LIMIT vanished and this deleted every matching node.
DO $$
DECLARE n int;
BEGIN
    SELECT count(*) INTO n
      FROM og_cypher('reg', 'MATCH (p:P) WHERE p.name = "a" RETURN p.name AS n');
    IF n <> 2 THEN
        RAISE EXCEPTION 'setup: expected 2 nodes named a, got %', n;
    END IF;

    PERFORM og_cypher('reg', 'MATCH (p:P) WHERE p.name = "a" WITH p LIMIT 1 DELETE p');

    SELECT count(*) INTO n
      FROM og_cypher('reg', 'MATCH (p:P) WHERE p.name = "a" RETURN p.name AS n');
    IF n <> 1 THEN
        RAISE EXCEPTION 'WITH ... LIMIT 1 DELETE removed % of 2 nodes', 2 - n;
    END IF;
END
$$;

\echo '--- and what cannot be honoured is refused, not ignored ---'
\echo 'OG_TEST_END'

-- EXPECT_ERROR: an aggregate in a WITH before a write reshapes the bindings,
-- so it is refused rather than dropped.
SELECT og_cypher('reg', 'MATCH (p:P) WITH count(p) AS c DELETE p');
