-- Run as supabase_admin right after pg_restore of AGE graphs (any database).
--
-- AGE keeps each graph's schema OID in ag_catalog.ag_graph.graphid (and in
-- ag_label.graph). pg_dump carries those numbers over unchanged, but the
-- restored schemas get new OIDs, so MATCH fails with
--   ERROR: graph with oid <old> does not exist
-- The namespace column (regnamespace) is restored by name and is already right,
-- so the fix is graphid := namespace::oid. This is the same remapping that
-- AGE's own age_finish_pg_upgrade() does after pg_upgrade.
-- Sessions that were open during the restore should reconnect afterwards.
BEGIN;
LOCK TABLE ag_catalog.ag_graph, ag_catalog.ag_label IN EXCLUSIVE MODE;

SELECT name, graphid AS old_graphid, namespace::oid AS new_graphid
  FROM ag_catalog.ag_graph WHERE graphid <> namespace::oid ORDER BY name;

ALTER TABLE ag_catalog.ag_label DROP CONSTRAINT IF EXISTS fk_graph_oid;

UPDATE ag_catalog.ag_label l
   SET graph = g.namespace::oid
  FROM ag_catalog.ag_graph g
 WHERE l.graph = g.graphid AND g.graphid <> g.namespace::oid;

UPDATE ag_catalog.ag_graph
   SET graphid = namespace::oid
 WHERE graphid <> namespace::oid;

ALTER TABLE ag_catalog.ag_label
  ADD CONSTRAINT fk_graph_oid FOREIGN KEY (graph) REFERENCES ag_catalog.ag_graph (graphid);

SELECT name, graphid, namespace, graphid = namespace::oid AS ok FROM ag_catalog.ag_graph ORDER BY name;
COMMIT;
