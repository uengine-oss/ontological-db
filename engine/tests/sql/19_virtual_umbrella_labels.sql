-- `(:_Entity:Customer)` over virtual types.
--
-- Applications put every class under one umbrella supertype (Ontology Studio's
-- `_Entity`) and write it next to the class label. A virtual type usually is
-- not under it — its rows live in the source, so it cannot be placed there by
-- the marker write a stored type gets — and the label set resolved to nothing:
-- an empty MATCH, and over a virtual relationship "needs both ends labelled".
-- Beside exactly one virtual type, an umbrella label is now set aside.
-- No expected errors here: any error fails the file.
\set ON_ERROR_STOP on

CREATE SCHEMA src19;
CREATE TABLE src19.customer (cust_id int PRIMARY KEY, name text);
CREATE TABLE src19.orders (order_id int PRIMARY KEY, cust_id int, amount int);
INSERT INTO src19.customer VALUES (1, 'Acme'), (2, 'Bolt'), (3, 'Core');
INSERT INTO src19.orders VALUES (10, 1, 5), (11, 1, 7), (12, 2, 9);

SELECT og_create_graph('umb');
-- The umbrella, with a stored class under it, as an application publishes it.
SELECT og_cypher('umb', 'CREATE (:_Entity:Note {text: ''x''})');
SELECT og_cypher('umb', 'CREATE (:Plain {x: 1})');
SELECT og_create_datasource('loop19', 'postgres', jsonb_build_object(
    'host', 'localhost', 'port', current_setting('port'), 'database', current_database(),
    'user', current_user, 'password', ''));
SELECT og_map_virtual('umb', 'Customer', 'loop19', 'src19.customer', 'cust_id');
SELECT og_map_virtual('umb', 'Order', 'loop19', 'src19.orders', 'order_id');
SELECT og_create_virtual_rel('umb', 'PLACED_BY', 'Order', 'cust_id', 'Customer', 'cust_id');

DO $$
DECLARE got jsonb; n int;
BEGIN
    SELECT og_cypher('umb', 'MATCH (o:Order)-[:PLACED_BY]->(c:_Entity:Customer) RETURN count(*) AS n') INTO got;
    IF (got->>'n')::int <> 3 THEN RAISE EXCEPTION 'umbrella on the far end: %', got; END IF;

    SELECT og_cypher('umb', 'MATCH (o:_Entity:Order)-[:PLACED_BY]->(c:_Entity:Customer {name: ''Acme''})
                             RETURN sum(o.amount) AS s') INTO got;
    IF (got->>'s')::int <> 12 THEN RAISE EXCEPTION 'umbrella on both ends: %', got; END IF;

    SELECT og_cypher('umb', 'MATCH (c:_Entity:Customer) RETURN count(c) AS n') INTO got;
    IF (got->>'n')::int <> 3 THEN RAISE EXCEPTION 'umbrella on a plain node pattern: %', got; END IF;

    -- The umbrella itself still means what it stores.
    SELECT og_cypher('umb', 'MATCH (n:_Entity) RETURN count(n) AS n') INTO got;
    IF (got->>'n')::int <> 1 THEN RAISE EXCEPTION ':_Entity alone: %', got; END IF;

    -- Only an umbrella is set aside. A label naming nothing, a concrete type
    -- with no subtypes, or a second virtual type still match nothing — and
    -- over a virtual relationship that is an empty result, not an error.
    SELECT count(*) INTO n FROM og_cypher('umb', 'MATCH (o:Order)-[:PLACED_BY]->(c:Nope:Customer) RETURN o');
    IF n <> 0 THEN RAISE EXCEPTION 'unknown label beside a virtual type matched % rows', n; END IF;
    SELECT count(*) INTO n FROM og_cypher('umb', 'MATCH (o:Order)-[:PLACED_BY]->(c:Plain:Customer) RETURN o');
    IF n <> 0 THEN RAISE EXCEPTION 'unrelated concrete label matched % rows', n; END IF;
    SELECT count(*) INTO n FROM og_cypher('umb', 'MATCH (o:Order)-[:PLACED_BY]->(c:Order:Customer) RETURN o');
    IF n <> 0 THEN RAISE EXCEPTION 'two virtual types matched % rows', n; END IF;

    -- Under OPTIONAL MATCH an impossible hop leaves NULLs, not an empty query.
    SELECT count(*) INTO n FROM og_cypher('umb',
        'MATCH (c:Customer) OPTIONAL MATCH (c)<-[:PLACED_BY]-(o:Nope:Order) RETURN c.name, o');
    IF n <> 3 THEN RAISE EXCEPTION 'OPTIONAL over an impossible hop: expected 3 rows, got %', n; END IF;
END
$$;
