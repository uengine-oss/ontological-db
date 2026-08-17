-- Deep-traversal bench fixture.
--
--   psql -v nodes=50000 -v degree=20 -f gen.sql
--
-- Same shape the main harness loads (bench/harness.py), but generated entirely
-- server-side so a million edges do not travel through a VALUES list, and from
-- a fixed seed so two runs are the same graph.
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

-- Uniform random edge list, fixed seed, no self loops, no duplicate (s,d).
SELECT setseed(0.42);
CREATE TEMP TABLE e_raw AS
SELECT DISTINCT s, d FROM (
  SELECT i AS s, (floor(random() * :nodes))::int AS d
    FROM generate_series(0, :nodes-1) i, generate_series(1, :degree) k
) x WHERE s <> d;

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

SELECT (SELECT count(*) FROM og_data.og_node) AS nodes,
       (SELECT count(*) FROM og_data.og_edge) AS edges,
       (SELECT count(*) FROM og_data.og_adj)  AS adj_segments;
