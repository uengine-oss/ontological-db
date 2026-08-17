-- Where pgGraph's time goes: traversal, or handing the rows back?
--
--   python3 bench/harness.py --scale 50000 --degree 20 --systems pggraph --hops 5
--   psql -d bench_pggraph -f bench/pggraph_cost.sql
--
-- Run this before quoting any pgGraph traversal number, including ours. The
-- headline figure for "nodes within k hops" turns out to be dominated by the
-- per-row cost of materialising the answer, not by walking the graph — and
-- pgGraph's own published figures are bounded traversals (depth 1 and 2), where
-- that cost is small. Comparing its full-result latency against a count from
-- another engine without saying so would be a strawman.
--
-- `max_rows` is the knob that separates the two: same traversal, same depth,
-- same everything, one of them asked to return ten rows instead of all of them.
\set ON_ERROR_STOP on
\pset pager off
\timing on

\set start '''7'''
\set depth 5

-- First call in a backend loads the .pggraph artifact into a private mapping.
-- That cost is real and is charged to whoever opens a connection, but it is not
-- the query's, so it is warmed away here and reported separately below.
SELECT count(*) FROM graph.traverse('public.n'::regclass, :start, max_depth := :depth,
    direction := 'out', uniqueness := 'node_global', include_start := false,
    hydrate := false, max_rows := 100000, max_nodes := 100000, max_frontier := 100000);

\echo ''
\echo '=== what this backend is holding ==='
SELECT node_count, edge_count, round(memory_used_mb::numeric, 2) AS mb,
       projection_mode, sync_mode FROM graph.status();

\echo ''
\echo '=== the whole answer: traversal + one row per reached node ==='
SELECT count(*) FROM graph.traverse('public.n'::regclass, :start, max_depth := :depth,
    direction := 'out', uniqueness := 'node_global', include_start := false,
    hydrate := false, max_rows := 100000, max_nodes := 100000, max_frontier := 100000);

\echo ''
\echo '=== the same traversal, ten rows returned: this is the CSR walk alone ==='
SELECT count(*) FROM graph.traverse('public.n'::regclass, :start, max_depth := :depth,
    direction := 'out', uniqueness := 'node_global', include_start := false,
    hydrate := false, max_rows := 10, max_nodes := 100000, max_frontier := 100000);

\echo ''
\echo '=== hydrate := true, for comparison — fetching the source rows is not the cost ==='
SELECT count(*) FROM graph.traverse('public.n'::regclass, :start, max_depth := :depth,
    direction := 'out', uniqueness := 'node_global', include_start := false,
    hydrate := true, max_rows := 100000, max_nodes := 100000, max_frontier := 100000);

\echo ''
\echo '=== deep search, tiny result: shortest path ==='
SELECT * FROM graph.shortest_path('public.n'::regclass, :start, 'public.n'::regclass, '1234');

\echo ''
\echo '=== cold: a brand-new backend pays for the artifact mapping ==='
\c
\timing on
SELECT count(*) FROM graph.traverse('public.n'::regclass, '7', max_depth := 1,
    direction := 'out', uniqueness := 'node_global', include_start := false,
    hydrate := false, max_rows := 100000, max_nodes := 100000, max_frontier := 100000);
SELECT count(*) FROM graph.traverse('public.n'::regclass, '7', max_depth := 1,
    direction := 'out', uniqueness := 'node_global', include_start := false,
    hydrate := false, max_rows := 100000, max_nodes := 100000, max_frontier := 100000);
