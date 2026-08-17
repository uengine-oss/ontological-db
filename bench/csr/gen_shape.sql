-- Deep-diameter fixtures.
--
--   psql -v shape=chain -v nodes=1000000 -f gen_shape.sql
--   psql -v shape=grid  -v nodes=1000000 -f gen_shape.sql     -- side = sqrt(nodes)
--
-- The random fixture in `gen.sql` is the wrong instrument for a question about
-- depth: at average degree 20 its whole graph is inside five hops, so "twenty
-- hops" and "eight hops" are the same question asked twice. Depth only means
-- something on a graph with a large diameter, and the two here are the shapes
-- that actually occur when someone needs one:
--
--   chain — 1,000,000 nodes in a line. Diameter = |V|. Out-degree 1, so the
--           frontier never grows and there is exactly one path to anywhere.
--           This is lineage, provenance, a supply chain, a reply thread. It
--           isolates *per-hop overhead* from frontier work, and it is the case
--           where trail enumeration costs nothing at all.
--
--   grid  — 1000 × 1000 lattice, each node pointing right and down. Diameter
--           1998, frontier grows linearly, nodes-within-k grows as k²/2 — and
--           the number of *paths* to a node at (i,j) is C(i+j, i), which is
--           combinatorial. Road networks, meshes, dependency DAGs. This is
--           where enumerating paths stops being possible while reachability
--           stays cheap.
\set ON_ERROR_STOP on

CREATE EXTENSION IF NOT EXISTS ontological CASCADE;
SELECT og_create_graph('benchg');
SELECT og_create_type('benchg','P','entity');
SELECT og_add_property('benchg','P','name','string');
SELECT og_add_property('benchg','P','val','int');
SELECT og_create_type('benchg','K','relation');

SELECT og_type_id('benchg','P') AS tid, og_type_id('benchg','K') AS rid \gset

INSERT INTO og_data.og_node (id, type_id)
SELECT og_make_id(0,:tid,i+1), :tid FROM generate_series(0,:nodes-1) i;

SELECT format($f$INSERT INTO og_data.n_%s (id, p_name, p_val)
                 SELECT og_make_id(0,%s,i+1), 'n'||i, i
                   FROM generate_series(0,%s) i$f$, :tid, :tid, :nodes-1) \gexec

-- Edge list. `val` is the node's ordinal, so a start node is addressed the same
-- way in every fixture and in every engine.
CREATE TEMP TABLE e_raw(s int, d int);

INSERT INTO e_raw(s, d)
SELECT i, i + 1 FROM generate_series(0, :nodes - 2) i
 WHERE :'shape' = 'chain';

-- Right and down on a square lattice; the last column has no right neighbour
-- and the last row has no down neighbour, so no edge leaves the grid.
INSERT INTO e_raw(s, d)
SELECT i, i + 1
  FROM generate_series(0, :nodes - 1) i
 WHERE :'shape' = 'grid'
   AND (i % (sqrt(:nodes)::int)) < (sqrt(:nodes)::int) - 1;

INSERT INTO e_raw(s, d)
SELECT i, i + (sqrt(:nodes)::int)
  FROM generate_series(0, :nodes - 1) i
 WHERE :'shape' = 'grid'
   AND i + (sqrt(:nodes)::int) < :nodes;

INSERT INTO og_data.og_edge (id, type_id, src, dst)
SELECT og_make_id(0,:rid, row_number() OVER ()), :rid,
       og_make_id(0,:tid, s+1), og_make_id(0,:tid, d+1) FROM e_raw;

SELECT format($f$INSERT INTO og_data.e_%s (id, src, dst)
                 SELECT id, src, dst FROM og_data.og_edge WHERE type_id = %s$f$,
              :rid, :rid) \gexec

INSERT INTO og_data.og_adj (src, etype, dir, seq, n, nbr, eid)
SELECT src, :rid, 'o', chunk, count(*)::int4, array_agg(dst), array_agg(id)
  FROM (SELECT src, dst, id,
               ((row_number() OVER (PARTITION BY src ORDER BY id)) - 1)::int4 / 256 AS chunk
          FROM og_data.og_edge WHERE type_id = :rid) x
 GROUP BY src, chunk;

INSERT INTO og_data.og_adj (src, etype, dir, seq, n, nbr, eid)
SELECT dst, :rid, 'i', chunk, count(*)::int4, array_agg(src), array_agg(id)
  FROM (SELECT src, dst, id,
               ((row_number() OVER (PARTITION BY dst ORDER BY id)) - 1)::int4 / 256 AS chunk
          FROM og_data.og_edge WHERE type_id = :rid) x
 GROUP BY dst, chunk;

SELECT og_create_index('benchg','P','val');
VACUUM ANALYZE;

SELECT :'shape' AS shape,
       (SELECT count(*) FROM og_data.og_node) AS nodes,
       (SELECT count(*) FROM og_data.og_edge) AS edges,
       round((SELECT count(*) FROM og_data.og_edge)::numeric
           / (SELECT count(*) FROM og_data.og_node), 2) AS avg_degree;
