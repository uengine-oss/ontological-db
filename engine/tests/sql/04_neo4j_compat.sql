-- Neo4j compatibility surface.
--
-- Every statement here is Cypher an application would send to Neo4j unchanged.
-- The point of the file is that none of it needs rewriting to run here.
\set ON_ERROR_STOP on
\pset pager off

SELECT og_create_graph('compat');
SELECT og_create_type('compat','_Entity','entity', ARRAY[]::text[], true);

\echo '=== implicit labels: writing a label declares it (Neo4j has no schema) ==='
SELECT og_cypher('compat', $$CREATE (n:_Entity:Doc {_source_id:'d1', title:'첫 문서'}) RETURN n$$);
SELECT og_cypher('compat', $$MERGE (n:_Entity:Doc {_source_id:'d2'})
    ON CREATE SET n.created_at = datetime()
    SET n.title = '둘째 문서', n.updated_at = datetime()
    RETURN n.title$$);

\echo '=== datetime() in a write clause records a real time, not null ==='
SELECT og_cypher('compat', $$MATCH (n:Doc {_source_id:'d2'}) RETURN n.created_at IS NOT NULL AS stamped$$);

\echo '=== multi-label patterns resolve to the most specific label ==='
SELECT og_cypher('compat', $$MATCH (n:_Entity:Doc) RETURN count(n) AS cnt$$);

\echo '=== a keyword can be a type name: CONTAINS keeps its spelling ==='
SELECT og_cypher('compat', $$MATCH (a:Doc {_source_id:'d1'}) MATCH (b:Doc {_source_id:'d2'})
    MERGE (a)-[r:CONTAINS]->(b) ON CREATE SET r.created_at = datetime() RETURN type(r) AS t$$);

\echo '=== elementId() round trips ==='
SELECT og_cypher('compat', $$MATCH (n:Doc) RETURN elementId(n) AS eid ORDER BY eid$$);

\echo '=== labels() is a list of the type and its supertypes ==='
SELECT og_cypher('compat', $$MATCH (n:Doc) RETURN labels(n) AS labels LIMIT 1$$);

\echo '=== list comprehension and list predicates ==='
SELECT og_cypher('compat', $$MATCH (n:_Entity)
    RETURN [l IN labels(n) WHERE l <> '_Entity'] AS classes LIMIT 1$$);
SELECT og_cypher('compat', $$MATCH (n:_Entity) WHERE any(l IN labels(n) WHERE l IN $classes)
    RETURN count(*) AS cnt$$, '{"classes":["Doc"]}');
SELECT og_cypher('compat', $$MATCH (n:_Entity) WHERE $class IN labels(n)
    RETURN count(*) AS cnt$$, '{"class":"Doc"}');

\echo '=== ORDER BY an aggregate alias ==='
SELECT og_cypher('compat', $$MATCH (n:_Entity)
    RETURN labels(n) AS class, count(*) AS count ORDER BY count DESC$$);

\echo '=== an unknown label matches nothing; it is not an error ==='
SELECT og_cypher('compat', $$MATCH (n:NeverWritten) RETURN count(n) AS cnt$$);
SELECT og_cypher('compat', $$MATCH (n:Doc) OPTIONAL MATCH (n)-[:NEVER_WRITTEN]->(m)
    RETURN count(n) AS kept$$);

\echo '=== WITH: the horizon, with aggregation and a post-aggregate filter ==='
SELECT og_cypher('compat', $$MATCH (n:_Entity)
    WITH labels(n) AS class, count(*) AS c
    WHERE c > 0
    RETURN class, c ORDER BY c DESC$$);
SELECT og_cypher('compat', $$MATCH (d:Doc)
    OPTIONAL MATCH (d)-[:CONTAINS]->(child:Doc)
    WITH d, collect(child._source_id) AS children
    RETURN d._source_id AS id, children ORDER BY id$$);

\echo '=== CREATE CONSTRAINT under its Neo4j spelling ==='
SELECT og_cypher('compat', $$CREATE CONSTRAINT doc_source_unique IF NOT EXISTS
    FOR (d:Doc) REQUIRE d._source_id IS UNIQUE$$);
-- idempotent, as IF NOT EXISTS promises
SELECT og_cypher('compat', $$CREATE CONSTRAINT doc_source_unique IF NOT EXISTS
    FOR (d:Doc) REQUIRE d._source_id IS UNIQUE$$);

\echo '=== CREATE VECTOR INDEX + db.index.vector.queryNodes ==='
SELECT og_cypher('compat', $$CREATE VECTOR INDEX doc_embedding IF NOT EXISTS
    FOR (n:_Entity) ON (n.embedding)
    OPTIONS {indexConfig: {`vector.dimensions`: 4, `vector.similarity_function`: 'cosine'}}$$);
SELECT og_cypher('compat', $$MATCH (n:Doc {_source_id:'d1'}) SET n.embedding = $v RETURN n._source_id$$,
    '{"v":[0.1,0.2,0.3,0.4]}');
SELECT og_cypher('compat', $$CALL db.index.vector.queryNodes('doc_embedding', 2, $v)
    YIELD node, score
    RETURN node._source_id AS id, labels(node) AS labels$$, '{"v":[0.1,0.2,0.3,0.4]}');

\echo '=== CREATE FULLTEXT INDEX + db.index.fulltext.queryNodes ==='
SELECT og_cypher('compat', $$CREATE FULLTEXT INDEX doc_text IF NOT EXISTS
    FOR (n:Doc) ON EACH [n.title]$$);
SELECT og_cypher('compat', $$CALL db.index.fulltext.queryNodes('doc_text', '문서')
    YIELD node, score RETURN count(node) AS hits$$);

\echo '=== apoc.neighbors.tohop feeding a later clause through WITH ==='
SELECT og_cypher('compat', $$MATCH (n:Doc {_source_id:'d1'})
    CALL apoc.neighbors.tohop(n, '>', 2) YIELD node AS m
    WITH n, m
    RETURN m._source_id AS reached ORDER BY reached$$);

\echo '=== db.labels / db.relationshipTypes ==='
SELECT og_cypher('compat', $$CALL db.labels() YIELD label RETURN label ORDER BY label$$);
SELECT og_cypher('compat', $$CALL db.relationshipTypes() YIELD relationshipType AS t RETURN t ORDER BY t$$);

\echo '=== renaming a class: REMOVE the old label, SET the new one ==='
SELECT og_cypher('compat', $$MATCH (n:_Entity:Doc) REMOVE n:Doc SET n:Document RETURN count(n) AS cnt$$);
SELECT og_cypher('compat', $$MATCH (n:Document) RETURN count(n) AS cnt$$);
-- and the old name is gone, so it matches nothing rather than erroring
SELECT og_cypher('compat', $$MATCH (n:Doc) RETURN count(n) AS cnt$$);

\echo '=== an unknown procedure is refused by name, not silently empty ==='
-- Caught in-band rather than left to the runner's error budget: an expected
-- error at the end of a file would also absorb an unexpected one before it, and
-- the file would pass while most of it never ran.
DO $do$
BEGIN
    PERFORM og_cypher('compat', $$CALL apoc.does.not.exist() YIELD x RETURN x$$);
    RAISE EXCEPTION 'an unknown procedure was accepted';
EXCEPTION WHEN others THEN
    IF position('is not available' in SQLERRM) = 0 THEN
        RAISE EXCEPTION 'wrong error for an unknown procedure: %', SQLERRM;
    END IF;
    RAISE NOTICE 'unknown procedure refused by name';
END
$do$;

\echo '=== the answers above are checked, not just printed ==='
-- A compatibility surface that runs and returns nothing is worse than one that
-- errors: the caller cannot tell it from "no matches". Every result the
-- procedures produced is asserted here.
DO $do$
DECLARE
    got jsonb;
BEGIN
    SELECT og_cypher('compat', $$MATCH (n:Document) RETURN count(n) AS c$$) INTO got;
    IF (got->>'c')::int <> 2 THEN
        RAISE EXCEPTION 'rename lost nodes: %', got;
    END IF;

    SELECT og_cypher('compat', $$CALL db.index.fulltext.queryNodes('doc_text', '문서')
        YIELD node RETURN count(node) AS hits$$) INTO got;
    IF (got->>'hits')::int <> 2 THEN
        RAISE EXCEPTION 'full-text search found % rows, expected 2', got->>'hits';
    END IF;

    SELECT og_cypher('compat', $$CALL db.index.vector.queryNodes('doc_embedding', 2, $v)
        YIELD node RETURN count(node) AS hits$$, '{"v":[0.1,0.2,0.3,0.4]}') INTO got;
    IF (got->>'hits')::int < 1 THEN
        RAISE EXCEPTION 'vector search found nothing';
    END IF;

    SELECT og_cypher('compat', $$MATCH (n:Document {_source_id:'d1'})
        CALL apoc.neighbors.tohop(n, '>', 2) YIELD node AS m
        RETURN count(m) AS reached$$) INTO got;
    IF (got->>'reached')::int <> 1 THEN
        RAISE EXCEPTION 'apoc.neighbors reached % nodes, expected 1', got->>'reached';
    END IF;

    -- apoc.meta.schema is what most Neo4j tooling calls first. Asserted on the
    -- three things a caller actually reads off it: the label is present, the
    -- count is a real count rather than a sample, and a relationship is
    -- reported from its source with a direction.
    SELECT og_cypher('compat', $$CALL apoc.meta.schema({sample: 1000})
        YIELD value RETURN value AS v$$) INTO got;
    IF got->'v'->'Document'->>'type' <> 'node' THEN
        RAISE EXCEPTION 'apoc.meta.schema did not report Document as a node: %', got;
    END IF;
    IF (got->'v'->'Document'->>'count')::int <> 2 THEN
        RAISE EXCEPTION 'apoc.meta.schema counted % Documents, expected 2',
            got->'v'->'Document'->>'count';
    END IF;
    IF got->'v'->'Document'->'properties'->'title'->>'type' <> 'STRING' THEN
        RAISE EXCEPTION 'apoc.meta.schema lost the declared type of Document.title: %',
            got->'v'->'Document'->'properties';
    END IF;
    IF got->'v'->'CONTAINS'->>'type' <> 'relationship' THEN
        RAISE EXCEPTION 'apoc.meta.schema did not report CONTAINS as a relationship: %',
            got->'v'->'CONTAINS';
    END IF;
    -- CONTAINS was created the Neo4j way, by writing it, so it has no declared
    -- roles and its direction can only come from the edges themselves.
    IF got->'v'->'Document'->'relationships'->'CONTAINS'->>'direction' <> 'out' THEN
        RAISE EXCEPTION 'apoc.meta.schema did not report CONTAINS leaving Document: %',
            got->'v'->'Document'->'relationships';
    END IF;
    IF NOT (got->'v'->'Document'->'relationships'->'CONTAINS'->'labels' ? 'Document') THEN
        RAISE EXCEPTION 'apoc.meta.schema lost the far end of CONTAINS: %',
            got->'v'->'Document'->'relationships'->'CONTAINS';
    END IF;

    RAISE NOTICE 'procedure results verified';
END
$do$;

\echo '=== write counters: what changed, counted as Neo4j counts it ==='
-- The Bolt gateway turns these into `summary.counters`, which is the only way a
-- driver learns that a write did anything. Checked on the create *and* the
-- delete, because a counter wired to one and not the other still looks right on
-- a single call.
DO $do$
DECLARE
    got jsonb;
BEGIN
    PERFORM og_cypher('compat', $$CREATE (n:Document {_source_id:'d9', title:'셋째'})$$);
    got := og_cypher_stats();
    IF (got->>'nodes-created')::int <> 1 OR (got->>'properties-set')::int <> 2
       OR NOT (got->>'contains-updates')::bool THEN
        RAISE EXCEPTION 'create counted %, expected 1 node and 2 properties', got;
    END IF;

    PERFORM og_cypher('compat', $$MATCH (n:Document {_source_id:'d9'}) DETACH DELETE n$$);
    got := og_cypher_stats();
    IF (got->>'nodes-deleted')::int <> 1 THEN
        RAISE EXCEPTION 'delete counted %, expected 1 node deleted', got;
    END IF;

    -- A read changes nothing, and must say so rather than repeating the last
    -- write's numbers.
    PERFORM og_cypher('compat', $$MATCH (n:Document) RETURN count(n) AS c$$);
    got := og_cypher_stats();
    IF (got->>'contains-updates')::bool THEN
        RAISE EXCEPTION 'a read reported updates: %', got;
    END IF;

    RAISE NOTICE 'write counters verified';
END
$do$;

\echo '=== genai.vector.encode is refused until it is configured ==='
-- The function reaches the network, so the interesting property to guard is
-- that it is *off*: a default that quietly made outbound requests would be the
-- bug. Encoding itself needs an endpoint and is exercised in
-- examples/meeting-rooms/, not here.
DO $do$
DECLARE
    failed boolean := false;
BEGIN
    BEGIN
        PERFORM og_cypher('compat', $$RETURN genai.vector.encode('probe') AS v$$);
    EXCEPTION WHEN OTHERS THEN
        failed := true;
        IF position('disabled' IN SQLERRM) = 0 THEN
            RAISE EXCEPTION 'expected a "disabled" refusal, got: %', SQLERRM;
        END IF;
    END;
    IF NOT failed THEN
        RAISE EXCEPTION 'genai.vector.encode ran without being enabled';
    END IF;
    RAISE NOTICE 'genai.vector.encode is off by default';
END
$do$;

\echo '=== DROP INDEX ==='
SELECT og_cypher('compat', $$DROP INDEX doc_text IF EXISTS$$);
SELECT og_cypher('compat', $$DROP INDEX doc_text IF EXISTS$$);

-- Reached only if every statement above succeeded. `ON_ERROR_STOP` aborts the
-- file on the first failure, so the marker is what distinguishes "all passed"
-- from "stopped early".
\echo '=== COMPAT SUITE COMPLETE ==='

\echo 'OG_TEST_END'
