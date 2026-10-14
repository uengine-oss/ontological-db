-- Virtual fabric: external rows as graph types, read in place.
--
-- The "external" database is this one, reached back through postgres_fdw — a
-- loopback keeps the suite self-contained while still exercising the real
-- path: foreign server, user mapping, IMPORT FOREIGN SCHEMA, remote SQL.
-- Asserts rather than prints; the expected errors are at the end.
\set ON_ERROR_STOP on

CREATE SCHEMA src;
CREATE TABLE src.supplier (code text PRIMARY KEY, name text, country text);
CREATE TABLE src.purchase (po_no text PRIMARY KEY, supplier_code text, amount numeric);
INSERT INTO src.supplier VALUES ('S1','Acme','KR'), ('S2','Bolt','CN'), ('S3','Core', NULL);
INSERT INTO src.purchase VALUES ('P1','S1',100), ('P2','S1',300), ('P3','S2',50), ('P4','S9',10);

SELECT og_create_graph('fab');
SELECT og_create_datasource('loop', 'postgres', jsonb_build_object(
    'host', 'localhost', 'port', current_setting('port'), 'database', current_database(),
    'user', current_user, 'password', ''));
SELECT og_map_virtual('fab', 'Supplier', 'loop', 'src.supplier', 'code');
SELECT og_map_virtual('fab', 'Purchase', 'loop', 'src.purchase', 'po_no');
SELECT og_create_virtual_rel('fab', 'FROM_SUPPLIER', 'Purchase', 'supplier_code', 'Supplier', 'code');

-- A native type that points at virtual rows by key.
SELECT og_create_type('fab', 'Risk', 'entity');
SELECT og_add_property('fab', 'Risk', 'title', 'string');
SELECT og_add_property('fab', 'Risk', 'supplier_code', 'string');
SELECT og_create_virtual_rel('fab', 'AFFECTS', 'Risk', 'supplier_code', 'Supplier', 'code');
SELECT og_cypher('fab', 'CREATE (:Risk {title:"flood", supplier_code:"S1"})');

DO $$
DECLARE n int; got jsonb; sql text;
BEGIN
    -- Label scan and a pushed filter.
    SELECT count(*) INTO n FROM og_cypher('fab', 'MATCH (s:Supplier) WHERE s.country = ''KR'' RETURN s.name');
    IF n <> 1 THEN RAISE EXCEPTION 'filter on a virtual type: expected 1, got %', n; END IF;

    -- A virtual hop, outgoing and incoming, and an unmatched key that must not join.
    SELECT count(*) INTO n FROM og_cypher('fab', 'MATCH (p:Purchase)-[:FROM_SUPPLIER]->(s:Supplier) RETURN p.po_no');
    IF n <> 3 THEN RAISE EXCEPTION 'virtual hop: expected 3 (P4 has no supplier), got %', n; END IF;
    SELECT og_cypher('fab', 'MATCH (s:Supplier {code:''S1''})<-[:FROM_SUPPLIER]-(p) RETURN sum(p.amount) AS t') INTO got;
    IF (got->>'t')::numeric <> 400 THEN RAISE EXCEPTION 'incoming hop with an inferred label: expected 400, got %', got; END IF;

    -- Native → virtual.
    SELECT og_cypher('fab', 'MATCH (r:Risk)-[:AFFECTS]->(s:Supplier)<-[:FROM_SUPPLIER]-(p:Purchase) RETURN count(p) AS c') INTO got;
    IF (got->>'c')::int <> 2 THEN RAISE EXCEPTION 'native → virtual → virtual: expected 2, got %', got; END IF;

    -- OPTIONAL MATCH keeps the supplier with no purchases.
    SELECT count(*) INTO n FROM og_cypher('fab',
        'MATCH (s:Supplier) OPTIONAL MATCH (s)<-[:FROM_SUPPLIER]-(p:Purchase) RETURN s.code, p.po_no');
    IF n <> 4 THEN RAISE EXCEPTION 'OPTIONAL over a virtual hop: expected 4 rows, got %', n; END IF;

    -- A relationship variable is returned with both ends.
    SELECT og_cypher('fab', 'MATCH (p:Purchase {po_no:''P3''})-[r:FROM_SUPPLIER]->(s:Supplier) RETURN r') INTO got;
    IF got->'r'->>'_type' <> 'FROM_SUPPLIER' OR got->'r'->'_src' IS NULL THEN
        RAISE EXCEPTION 'virtual relationship value: %', got;
    END IF;

    -- The unmatched end of an OPTIONAL MATCH is null, so count() skips it:
    -- suppliers with no purchase are found by counting to zero.
    SELECT count(*) INTO n FROM og_cypher('fab',
        'MATCH (s:Supplier) OPTIONAL MATCH (s)<-[:FROM_SUPPLIER]-(p:Purchase) WITH s, count(p) AS c WHERE c = 0 RETURN s.code');
    IF n <> 1 THEN RAISE EXCEPTION 'anti-join through OPTIONAL MATCH: expected 1 (S3), got %', n; END IF;
    SELECT og_cypher('fab', 'MATCH (s:Supplier {code:''S3''}) OPTIONAL MATCH (s)<-[:FROM_SUPPLIER]-(p:Purchase) RETURN p') INTO got;
    IF got->'p' <> 'null'::jsonb THEN RAISE EXCEPTION 'unmatched optional node should be null, got %', got; END IF;

    -- A stored edge can hang off a virtual node.
    PERFORM og_cypher('fab', 'MATCH (s:Supplier {code:''S2''}), (r:Risk) CREATE (r)-[:WATCHES]->(s)');
    SELECT count(*) INTO n FROM og_cypher('fab', 'MATCH (r:Risk)-[:WATCHES]->(s:Supplier) RETURN s.name');
    IF n <> 1 THEN RAISE EXCEPTION 'stored edge onto a virtual node: expected 1, got %', n; END IF;

    -- What reached the source is reported, per step.
    SELECT count(*) INTO n FROM og_fabric_explain('fab',
        'MATCH (p:Purchase)-[:FROM_SUPPLIER]->(s:Supplier) WHERE s.country = ''KR'' RETURN p.po_no', true)
     WHERE place = 'remote:loop' AND detail LIKE '%JOIN%';
    IF n <> 1 THEN RAISE EXCEPTION 'same-server hop should be one pushed-down join, got % such steps', n; END IF;

    -- Equality against a constant is plain `=`: null = 'x' is null, so a
    -- supplier with no country is neither equal nor unequal to 'KR'.
    SELECT count(*) INTO n FROM og_cypher('fab', 'MATCH (s:Supplier) WHERE s.country <> ''KR'' RETURN s.code');
    IF n <> 1 THEN RAISE EXCEPTION '<> against a constant: expected 1 (S2; S3 is null), got %', n; END IF;

    -- The filter, IN list included, reaches the source rather than running here.
    SELECT og_cypher_sql('fab', 'MATCH (s:Supplier) WHERE s.country IN [''KR'',''CN''] RETURN s.code') INTO sql;
    EXECUTE 'EXPLAIN (VERBOSE, COSTS OFF) ' || sql;
    IF sql NOT LIKE '%IN (%' THEN RAISE EXCEPTION 'IN over literals should compile to SQL IN: %', sql; END IF;
END
$$;

-- Several refusals follow, and each has to be reached.
\set ON_ERROR_STOP off

-- EXPECT_ERROR: virtual types are read-only, whatever the source account may do.
SELECT og_cypher('fab', 'CREATE (:Supplier {code:"S9", name:"x"})');
-- EXPECT_ERROR: SET on a virtual node is refused before it reaches the source.
SELECT og_cypher('fab', 'MATCH (s:Supplier {code:"S3"}) SET s.name = "y"');
-- EXPECT_ERROR: so is DELETE — the loopback account here could delete the row.
SELECT og_cypher('fab', 'MATCH (s:Supplier {code:"S3"}) DELETE s');
SELECT 1 / (SELECT count(*) FILTER (WHERE code = 'S3' AND name = 'Core') FROM src.supplier) AS source_untouched;

-- EXPECT_ERROR: a virtual relationship is a join, so it has no variable-length form yet.
SELECT og_cypher('fab', 'MATCH (p:Purchase)-[:FROM_SUPPLIER*1..2]->(s:Supplier) RETURN s');
