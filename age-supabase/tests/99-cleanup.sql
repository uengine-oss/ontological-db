-- Run as robo: remove the test graph so the database is left as the image made it.
BEGIN;
SET LOCAL search_path = ag_catalog, "$user", public;
SELECT drop_graph(name, true) FROM ag_catalog.ag_graph WHERE name IN ('robo_t1', 'agesb_rust');
COMMIT;
