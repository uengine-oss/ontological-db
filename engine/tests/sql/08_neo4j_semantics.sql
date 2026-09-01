-- Regressions for thirteen defects found by running an application on the Bolt
-- gateway rather than by reading the docs.
--
-- Four were syntax errors and announced themselves. The rest returned a wrong
-- answer with no error at all — the screen rendered, the numbers were wrong —
-- which is why they are asserted here rather than printed. Every check below
-- fails on 3179cc7.
--
-- Two of them only exist because reads and writes take different code: a read
-- compiles to SQL, while a write compiles its read clauses and then evaluates
-- the write clauses per row in Rust. A function has to be mapped in both.
\set ON_ERROR_STOP on
\pset pager off

SELECT og_create_graph('sem');
SELECT og_create_type('sem', 'Doc', 'entity');
SELECT og_add_property('sem', 'Doc', 'id', 'string');
SELECT og_add_property('sem', 'Doc', 'title', 'string');
SELECT og_add_property('sem', 'Doc', 'tags', 'string');
SELECT og_add_property('sem', 'Doc', 'at', 'string');
SELECT og_create_type('sem', 'Tag', 'entity');
SELECT og_add_property('sem', 'Tag', 'id', 'string');
SELECT og_add_property('sem', 'Tag', 'name', 'string');
SELECT og_create_type('sem', 'HAS_TAG', 'relation');

SELECT og_cypher('sem', $$CREATE (:Doc {id:'D-1', title:'first'})$$);
SELECT og_cypher('sem', $$CREATE (:Doc {id:'D-2', title:'second'})$$);
SELECT og_cypher('sem', $$CREATE (:Tag {id:'T-1', name:'blue'})$$);
SELECT og_cypher('sem', $$CREATE (:Doc {id:'D-3', title:'plain'})$$);
SELECT og_cypher('sem', $$MATCH (d:Doc), (t:Tag) WHERE d.title = 'first'
                          CREATE (d)-[:HAS_TAG]->(t)$$);

\echo '--- 1. n.id is the user property, not the internal row id ---'
-- `id` was short-circuited to the internal bigint before the property catalog
-- was consulted. Writing it worked, reading it gave 549755813889, and matching
-- on it raised a type error. Neo4j reserves id(n) for the internal one.
DO $$
DECLARE v text; n int;
BEGIN
    SELECT (og_cypher('sem', $q$MATCH (d:Doc) WHERE d.title = 'first'
                                RETURN d.id AS id$q$)->>'id') INTO v;
    IF v IS DISTINCT FROM 'D-1' THEN
        RAISE EXCEPTION 'n.id: expected D-1, got %', v;
    END IF;
    SELECT count(*) INTO n FROM og_cypher('sem', $q$MATCH (d:Doc {id:'D-1'}) RETURN d$q$);
    IF n <> 1 THEN
        RAISE EXCEPTION 'matching on id: expected 1 row, got %', n;
    END IF;
END $$;

\echo '--- 2. OPTIONAL MATCH correlates when it adds more than one join ---'
-- A clause that adds two joins left its first one as `ON true`, so it paired
-- every outer row with every inner one. The shape needs the optional pattern to
-- lead with a node that is not yet bound: then the correlation to the outer row
-- sits at the far end of the clause, and only the joins after the first got a
-- predicate. An optional pattern that starts from an already-bound node was
-- fine, which is why this survived.
DO $$
DECLARE n int; v text;
BEGIN
    SELECT count(*) INTO n FROM og_cypher('sem',
        $q$MATCH (t:Tag) OPTIONAL MATCH (d:Doc)-[:HAS_TAG]->(t)
           RETURN t.name AS t, d.title AS d$q$);
    IF n <> 1 THEN
        RAISE EXCEPTION 'OPTIONAL MATCH: expected 1 row (the one tagged Doc), got %', n;
    END IF;

    SELECT (og_cypher('sem',
        $q$MATCH (t:Tag) OPTIONAL MATCH (d:Doc)-[:HAS_TAG]->(t)
           RETURN d.title AS d$q$)->>'d') INTO v;
    IF v IS DISTINCT FROM 'first' THEN
        RAISE EXCEPTION 'OPTIONAL MATCH paired the wrong rows: got %', coalesce(v, 'NULL');
    END IF;
END $$;

\echo '--- 3. an OPTIONAL MATCH that misses yields NULL, not an empty object ---'
-- jsonb_strip_nulls folded an all-NULL row into {}, which is not SQL NULL. So
-- `t.id` was right while `t IS NULL` was false and `count(t)` counted the miss.
-- Anti-joins, existence tests and count() all read the wrong answer at once.
DO $$
DECLARE n int;
BEGIN
    SELECT count(*) INTO n FROM og_cypher('sem',
        $q$MATCH (d:Doc) OPTIONAL MATCH (d)-[:HAS_TAG]->(t:Tag)
           WITH d, t WHERE t IS NULL RETURN d.title AS d$q$);
    IF n <> 2 THEN
        RAISE EXCEPTION 'IS NULL on a missed OPTIONAL MATCH: expected 2 rows, got %', n;
    END IF;

    SELECT (og_cypher('sem',
        $q$MATCH (d:Doc) OPTIONAL MATCH (d)-[:HAS_TAG]->(t:Tag)
           RETURN count(t) AS c$q$)->>'c')::int INTO n;
    IF n <> 1 THEN
        RAISE EXCEPTION 'count() over a missed OPTIONAL MATCH: expected 1, got %', n;
    END IF;
END $$;

\echo '--- 4. a label predicate is allowed in WHERE ---'
-- Syntax error: the AST had no such node. The idiom appears wherever one query
-- has to handle several kinds of target.
DO $$
DECLARE n int;
BEGIN
    SELECT count(*) INTO n FROM og_cypher('sem',
        $q$MATCH (x) WHERE (x:Doc AND x.id = 'D-1') OR (x:Tag) RETURN x$q$);
    IF n <> 2 THEN
        RAISE EXCEPTION 'label predicate in WHERE: expected 2 rows, got %', n;
    END IF;
END $$;

\echo '--- 5. an untyped MATCH stays inside its graph ---'
-- The graph a node belongs to is encoded in its type_id, so a scan with a label
-- lands in a per-type view and is confined. A scan without one read og_node
-- whole and crossed every graph in the database. Projects are graphs here, so
-- this was the isolation boundary leaking.
SELECT og_create_graph('sem_other');
SELECT og_create_type('sem_other', 'Elsewhere', 'entity');
SELECT og_add_property('sem_other', 'Elsewhere', 'id', 'string');
SELECT og_cypher('sem_other', $$CREATE (:Elsewhere {id:'X-1'})$$);

DO $$
DECLARE here int; there int;
BEGIN
    SELECT (og_cypher('sem',       $q$MATCH (n) RETURN count(n) AS c$q$)->>'c')::int INTO here;
    SELECT (og_cypher('sem_other', $q$MATCH (n) RETURN count(n) AS c$q$)->>'c')::int INTO there;
    IF here <> 4 OR there <> 1 THEN
        RAISE EXCEPTION 'untyped scan crossed graphs: sem=% (want 4), sem_other=% (want 1)',
              here, there;
    END IF;
END $$;

\echo '--- 6. randomUUID() produces a value inside SET ---'
-- `RETURN randomUUID()` failed loudly with `unknown function`, while the same
-- call inside SET quietly wrote NULL: write-clause values are evaluated in Rust,
-- not compiled to SQL, and only the compiler knew the name. The node was created
-- and only its id was empty, so the application saw "returned empty result".
DO $$
DECLARE v text;
BEGIN
    SELECT (og_cypher('sem', $q$CREATE (d:Doc {title:'uuid'})
                                SET d.id = randomUUID() RETURN d.id AS id$q$)->>'id') INTO v;
    IF v IS NULL OR length(v) < 30 THEN
        RAISE EXCEPTION 'randomUUID() in SET: expected a uuid, got %', coalesce(v, 'NULL');
    END IF;
    SELECT (og_cypher('sem', $q$RETURN randomUUID() AS id$q$)->>'id') INTO v;
    IF v IS NULL OR length(v) < 30 THEN
        RAISE EXCEPTION 'randomUUID() in RETURN: expected a uuid, got %', coalesce(v, 'NULL');
    END IF;
END $$;

\echo '--- 7. a pattern predicate is allowed in WHERE ---'
-- Also a syntax error. Compiles to a correlated EXISTS: a node already bound
-- outside keeps its alias, so the subquery correlates on its own.
DO $$
DECLARE with_tag int; without int;
BEGIN
    SELECT count(*) INTO with_tag FROM og_cypher('sem',
        $q$MATCH (d:Doc) WHERE (d)-[:HAS_TAG]->(:Tag) RETURN d.title AS d$q$);
    SELECT count(*) INTO without FROM og_cypher('sem',
        $q$MATCH (d:Doc) WHERE NOT (d)-[:HAS_TAG]->(:Tag) RETURN d.title AS d$q$);
    IF with_tag <> 1 OR without <> 3 THEN
        RAISE EXCEPTION 'pattern predicate: expected 1 with and 3 without, got % and %',
              with_tag, without;
    END IF;
END $$;

\echo '--- 8. a WITH before a write may carry a computed item ---'
-- Refused on purpose, but too widely. What a write clause cannot survive is a
-- projection that renames a pattern variable, because it names those variables.
-- A computed item under a new alias shadows nothing.
DO $$
DECLARE n int;
BEGIN
    SELECT (og_cypher('sem',
        $q$MATCH (d:Doc) WHERE d.title = 'first' OPTIONAL MATCH (d)-[:HAS_TAG]->(t:Tag)
           WITH d, count(t) AS tags SET d.title = 'counted' RETURN d.title AS t$q$)
        IS NOT NULL)::int INTO n;
    IF n <> 1 THEN
        RAISE EXCEPTION 'computed item in a WITH before a write was refused';
    END IF;
END $$;

\echo '--- 9. coalesce() over a property and a list literal agrees on a type ---'
-- Property reads came back as text and list literals as jsonb, and COALESCE
-- will not mix them. The result of coalesce(tags, []) has to be a list.
DO $$
DECLARE v text;
BEGIN
    SELECT (og_cypher('sem', $q$MATCH (d:Doc) WHERE d.title = 'plain'
                                RETURN coalesce(d.tags, []) AS tags$q$)->>'tags') INTO v;
    IF v IS DISTINCT FROM '[]' THEN
        RAISE EXCEPTION 'coalesce(prop, []): expected [], got %', coalesce(v, 'NULL');
    END IF;
END $$;

\echo '--- 10. datetime() reads back in the format it was written in ---'
-- The read path compiled to SQL and produced ISO 8601; the write path evaluated
-- in Rust and produced PostgreSQL''s own rendering. The same call gave two
-- formats depending on where it appeared, and ISO parsers downstream broke on
-- one of them. Neo4j is ISO everywhere.
DO $$
DECLARE written text; returned text;
BEGIN
    SELECT (og_cypher('sem', $q$MATCH (d:Doc) WHERE d.title = 'second' SET d.at = datetime()
                                RETURN d.at AS at$q$)->>'at') INTO written;
    SELECT (og_cypher('sem', $q$RETURN datetime() AS at$q$)->>'at') INTO returned;
    IF written !~ '^\d{4}-\d{2}-\d{2}T' THEN
        RAISE EXCEPTION 'datetime() in SET: expected ISO 8601, got %', written;
    END IF;
    IF returned !~ '^\d{4}-\d{2}-\d{2}T' THEN
        RAISE EXCEPTION 'datetime() in RETURN: expected ISO 8601, got %', returned;
    END IF;
END $$;

\echo '--- 11. + concatenates lists ---'
-- Cypher''s + joins lists; jsonb''s concatenation operator is ||, and jsonb has
-- no + at all, so this failed at execution.
DO $$
DECLARE v text;
BEGIN
    SELECT (og_cypher('sem', $q$RETURN [1,2] + [3] AS l$q$)->>'l') INTO v;
    IF v IS DISTINCT FROM '[1, 2, 3]' AND v IS DISTINCT FROM '[1,2,3]' THEN
        RAISE EXCEPTION 'list concatenation: expected [1,2,3], got %', coalesce(v, 'NULL');
    END IF;
END $$;

\echo '--- 12. collect() over nothing is an empty list ---'
-- jsonb_agg returns NULL over no rows; Cypher returns []. A caller that expects
-- a list and iterates it fails on the empty database — the first screen after
-- a fresh install, which is exactly where this was found.
DO $$
DECLARE v text;
BEGIN
    SELECT (og_cypher('sem', $q$MATCH (n:Tag) WHERE n.name = 'nope'
                                RETURN collect(n.id) AS c$q$)->>'c') INTO v;
    IF v IS DISTINCT FROM '[]' THEN
        RAISE EXCEPTION 'collect() over no rows: expected [], got %', coalesce(v, 'NULL');
    END IF;
END $$;

\echo '--- 13. toString() of a value that crossed a WITH is the scalar ---'
-- Everything crossing a WITH becomes jsonb, and casting jsonb to text yields its
-- JSON rendering — quotes included. The quotes travelled into the response and
-- broke date parsing on the other side.
DO $$
DECLARE v text;
BEGIN
    SELECT (og_cypher('sem', $q$MATCH (d:Doc) WHERE d.title = 'plain' WITH d.title AS t
                                RETURN toString(t) AS s$q$)->>'s') INTO v;
    IF v IS DISTINCT FROM 'plain' THEN
        RAISE EXCEPTION 'toString() after a WITH: expected plain, got %', coalesce(v, 'NULL');
    END IF;
END $$;

\echo '--- 14. CREATE INDEX works on a property nothing has written yet ---'
-- Declaring indexes at startup, before the first node exists, is what an
-- application does. Only the b-tree branch skipped the "declare the property so
-- there is a column to index" step that the full-text and constraint paths both
-- take, so this failed with `column "p_lang" does not exist` until something
-- happened to write that property first — which made it look like an ordering
-- quirk rather than a missing call.
SELECT og_create_type('sem', 'Fresh2', 'entity');

DO $$
DECLARE n int;
BEGIN
    PERFORM og_cypher('sem',
        'CREATE INDEX fresh2_lang IF NOT EXISTS FOR (f:Fresh2) ON (f.lang)');
    SELECT count(*) INTO n FROM og_catalog.property p
      JOIN og_catalog.type t ON t.type_id = p.type_id
     WHERE t.name = 'Fresh2' AND p.name = 'lang';
    IF n <> 1 THEN
        RAISE EXCEPTION 'CREATE INDEX did not declare the property: got % rows', n;
    END IF;
    -- A second call must pass quietly under IF NOT EXISTS.
    PERFORM og_cypher('sem',
        'CREATE INDEX fresh2_lang IF NOT EXISTS FOR (f:Fresh2) ON (f.lang)');
END $$;

\echo 'OG_TEST_END'
