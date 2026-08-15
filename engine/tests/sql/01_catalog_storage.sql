-- Spec 001 + 002 regression: type hierarchy, interval labels, storage, adjacency.
\set ON_ERROR_STOP on

SELECT og_create_graph('demo');

-- Inheritance: EV ⊂ Car ⊂ Vehicle, Truck ⊂ Vehicle
SELECT og_create_type('demo', 'Vehicle', 'entity');
SELECT og_create_type('demo', 'Car',     'entity', ARRAY['Vehicle']);
SELECT og_create_type('demo', 'Truck',   'entity', ARRAY['Vehicle']);
SELECT og_create_type('demo', 'EV',      'entity', ARRAY['Car']);
SELECT og_create_type('demo', 'Person',  'entity');

SELECT og_add_property('demo', 'Vehicle', 'model', 'string');
SELECT og_add_property('demo', 'Vehicle', 'year',  'int');
SELECT og_add_property('demo', 'EV',      'range_km', 'int');
SELECT og_add_property('demo', 'Person',  'name',  'string', required => true);

SELECT og_create_type('demo', 'OWNS', 'relation');
SELECT og_add_role('demo', 'OWNS', 'owner',  'Person',  0);
SELECT og_add_role('demo', 'OWNS', 'owned',  'Vehicle', 1);

\echo '--- type hierarchy labels (no recursion anywhere) ---'
SELECT name, depth, lft, rgt, parents FROM og_type_view WHERE graph='demo' ORDER BY lft;

\echo '--- subtype resolution ---'
SELECT og_is_subtype(og_type_id('demo','EV'),      og_type_id('demo','Vehicle')) AS ev_is_vehicle,
       og_is_subtype(og_type_id('demo','Vehicle'), og_type_id('demo','EV'))      AS vehicle_is_ev,
       og_is_subtype(og_type_id('demo','Truck'),   og_type_id('demo','Car'))     AS truck_is_car;

\echo '--- instances ---'
SELECT og_create_node('demo', 'Car',   '{"model":"Model 3 base","year":2019}') AS car1 \gset
SELECT og_create_node('demo', 'EV',    '{"model":"Model 3","year":2024,"range_km":600}') AS ev1 \gset
SELECT og_create_node('demo', 'Truck', '{"model":"F-150","year":2022}') AS tr1 \gset
SELECT og_create_node('demo', 'Person', '{"name":"Aria","nickname":"ari"}') AS p1 \gset

\echo '--- properties are REAL COLUMNS, not JSON blobs ---'
SELECT column_name, data_type FROM information_schema.columns
 WHERE table_schema='og_data' AND table_name = 'n_' || og_type_id('demo','EV')
 ORDER BY ordinal_position;

\echo '--- ad-hoc property landed in __ext, declared ones did not ---'
SELECT p_name, __ext FROM og_data.og_node n
  JOIN og_catalog.type t ON t.type_id=n.type_id, LATERAL (SELECT 1) z
  , og_data.n_5 x WHERE x.id = n.id AND t.name='Person';

\echo '--- MATCH (v:Vehicle) finds every descendant type ---'
SELECT count(*) AS vehicles FROM og_nodes(og_type_id('demo','Vehicle'));
SELECT count(*) AS cars     FROM og_nodes(og_type_id('demo','Car'));
SELECT count(*) AS evs      FROM og_nodes(og_type_id('demo','EV'));

\echo '--- edges + adjacency ---'
SELECT og_create_edge('demo','OWNS', :p1, :ev1, '{}') AS e1 \gset
SELECT og_create_edge('demo','OWNS', :p1, :tr1, '{}') AS e2 \gset

SELECT src, etype, dir, seq, n, array_length(nbr,1) AS len FROM og_data.og_adj ORDER BY src, dir;

\echo '--- expand ---'
SELECT nbr, eid FROM og_expand(:p1, NULL, 'o') ORDER BY nbr;
SELECT og_degree_all(:p1, 'o') AS out_degree;

\echo '--- role constraint is enforced ---'
-- EXPECT_ERROR: an EV cannot play the 'owner' role
SELECT og_create_edge('demo','OWNS', :ev1, :p1, '{}');
