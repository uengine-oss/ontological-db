-- Run as robo after 10-robo-cypher.sql: parameters of cypher().
\pset footer off
SET search_path = ag_catalog, "$user", public;

\echo '### P1 PREPARE with an explicit agtype parameter'
PREPARE p1(ag_catalog.agtype) AS
  SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (a:Aggregate) WHERE a.name = $name RETURN a.name, a.version $$, $1)
    AS (n ag_catalog.agtype, v ag_catalog.agtype);
EXECUTE p1('{"name": "Order"}');
EXECUTE p1('{"name": "OrderLine"}');

\echo '### P2 PREPARE without a declared type: the server infers agtype'
PREPARE p2 AS
  SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (a:Aggregate {name: $name}) RETURN a.name $$, $1) AS (n ag_catalog.agtype);
SELECT name, parameter_types FROM pg_prepared_statements WHERE name = 'p2';
EXECUTE p2('{"name": "Order"}');

\echo '### P3 parameter of type text (or a cast) is refused: the third argument must be a bare Param'
PREPARE p3(text) AS
  SELECT * FROM ag_catalog.cypher('robo_t1', $$ RETURN $x $$, $1::ag_catalog.agtype) AS (x ag_catalog.agtype);
PREPARE p3b(jsonb) AS
  SELECT * FROM ag_catalog.cypher('robo_t1', $$ RETURN $x $$, $1::text::ag_catalog.agtype) AS (x ag_catalog.agtype);

\echo '### P4 a literal map instead of a parameter is refused'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ RETURN $x $$, '{"x": 1}') AS (x ag_catalog.agtype);

\echo '### P5 graph name or query as parameters are refused (must be constants)'
PREPARE p5(name) AS SELECT * FROM ag_catalog.cypher($1, $$ RETURN 1 $$) AS (x ag_catalog.agtype);
PREPARE p5b(cstring) AS SELECT * FROM ag_catalog.cypher('robo_t1', $1) AS (x ag_catalog.agtype);

\echo '### P6 query text as an ordinary string constant (not dollar-quoted)'
SELECT * FROM ag_catalog.cypher('robo_t1', 'MATCH (a:Aggregate) WHERE a.name = ''Order'' RETURN a.name') AS (n ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', E'MATCH (a:Aggregate)\nRETURN count(a)') AS (n ag_catalog.agtype);
\echo '-- a custom dollar-quote tag works too (what a gateway should use for client text)'
SELECT * FROM ag_catalog.cypher('robo_t1', $cy_7f3a$ RETURN 'contains $$ inside' $cy_7f3a$) AS (n ag_catalog.agtype);

\echo '### P7 parameters in writes: scalars, lists, maps, SET +=, UNWIND of a list of maps'
PREPARE w1(ag_catalog.agtype) AS
  SELECT * FROM ag_catalog.cypher('robo_t1', $$
    CREATE (c:Command {name: $name, tags: $tags, spec: $spec}) RETURN c
  $$, $1) AS (c ag_catalog.agtype);
EXECUTE w1('{"name": "PlaceOrder", "tags": ["write", "sales"], "spec": {"idempotent": true, "retries": 3}}');
PREPARE w2(ag_catalog.agtype) AS
  SELECT * FROM ag_catalog.cypher('robo_t1', $$
    MATCH (c:Command {name: $name}) SET c += $patch RETURN c
  $$, $1) AS (c ag_catalog.agtype);
EXECUTE w2('{"name": "PlaceOrder", "patch": {"owner": "team-b", "version": 2}}');
PREPARE w3(ag_catalog.agtype) AS
  SELECT * FROM ag_catalog.cypher('robo_t1', $$
    UNWIND $rows AS row CREATE (c:Command) SET c = row RETURN c.name
  $$, $1) AS (n ag_catalog.agtype);
EXECUTE w3('{"rows": [{"name": "CancelOrder", "k": 1}, {"name": "ShipOrder", "k": 2}]}');
\echo '-- a whole map as the property map of a pattern: CREATE (c:Command $props)'
PREPARE w4(ag_catalog.agtype) AS
  SELECT * FROM ag_catalog.cypher('robo_t1', $$ CREATE (c:Command $props) RETURN c $$, $1) AS (c ag_catalog.agtype);
EXECUTE w4('{"props": {"name": "RefundOrder"}}');
\echo '-- workarounds: bind the map with WITH first, or SET n = $map after CREATE'
PREPARE w2b(ag_catalog.agtype) AS
  SELECT * FROM ag_catalog.cypher('robo_t1', $$
    WITH $patch AS patch MATCH (c:Command {name: $name}) SET c += patch RETURN c
  $$, $1) AS (c ag_catalog.agtype);
EXECUTE w2b('{"name": "PlaceOrder", "patch": {"owner": "team-b", "version": 2}}');
PREPARE w4b(ag_catalog.agtype) AS
  SELECT * FROM ag_catalog.cypher('robo_t1', $$ CREATE (c:Command) SET c = $props RETURN c $$, $1) AS (c ag_catalog.agtype);
EXECUTE w4b('{"props": {"name": "RefundOrder"}}');
PREPARE w4c(ag_catalog.agtype) AS
  SELECT * FROM ag_catalog.cypher('robo_t1', $$ WITH $props AS p CREATE (c:Command) SET c = p RETURN c $$, $1) AS (c ag_catalog.agtype);
EXECUTE w4c('{"props": {"name": "ReturnOrder"}}');
\echo '-- edge with parameters between matched nodes'
PREPARE w5(ag_catalog.agtype) AS
  SELECT * FROM ag_catalog.cypher('robo_t1', $$
    MATCH (a:Command {name: $from}), (b:Command {name: $to}) CREATE (a)-[r:TRIGGERS {why: $why}]->(b) RETURN r
  $$, $1) AS (r ag_catalog.agtype);
EXECUTE w5('{"from": "PlaceOrder", "to": "ShipOrder", "why": "flow"}');
\echo '-- LIMIT / SKIP from a parameter'
PREPARE r1(ag_catalog.agtype) AS
  SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (c:Command) RETURN c.name ORDER BY c.name SKIP $s LIMIT $l $$, $1) AS (n ag_catalog.agtype);
EXECUTE r1('{"s": 1, "l": 2}');
\echo '-- a parameter that is not in the map is null; extra keys are ignored'
PREPARE r2(ag_catalog.agtype) AS
  SELECT * FROM ag_catalog.cypher('robo_t1', $$ RETURN $missing IS NULL, $present $$, $1) AS (a ag_catalog.agtype, b ag_catalog.agtype);
EXECUTE r2('{"present": 1, "extra": 2}');
\echo '-- $param used but NULL passed for the whole map'
EXECUTE r2(NULL);
\echo '-- a query that names $params but is called without the third argument'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ RETURN $x $$) AS (x ag_catalog.agtype);

\echo '### P8 plpgsql EXECUTE ... USING (the PostgREST wrapper style)'
\echo '-- %L (ordinary literal) is refused; a dollar-quote tag that the text does not contain works'
DO $do$
DECLARE r text; q text := 'MATCH (c:Command {name: $n}) RETURN c.name'; tag text := '$cy_' || md5(random()::text) || '$';
BEGIN
  BEGIN
    EXECUTE format('SELECT x::text FROM ag_catalog.cypher(%L, %L, $1) AS (x ag_catalog.agtype)', 'robo_t1', q)
       INTO r USING ('{"n": "PlaceOrder"}')::ag_catalog.agtype;
  EXCEPTION WHEN others THEN RAISE NOTICE '%%L form -> ERROR: %', SQLERRM;
  END;
  IF position(tag IN q) > 0 THEN RAISE EXCEPTION 'tag collision'; END IF;
  EXECUTE format('SELECT x::text FROM ag_catalog.cypher(%L, %s%s%s, $1) AS (x ag_catalog.agtype)', 'robo_t1', tag, q, tag)
     INTO r USING ('{"n": "PlaceOrder"}')::ag_catalog.agtype;
  RAISE NOTICE 'dollar-tag form -> %', r;
END
$do$;

\echo '### P9 cleanup of the parameter test data'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (c:Command) DETACH DELETE c RETURN count(*) $$) AS (c ag_catalog.agtype);
DEALLOCATE ALL;
