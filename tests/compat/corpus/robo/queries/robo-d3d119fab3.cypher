MATCH (ui:UI)-[a:ATTACHED_TO]->(rm:ReadModel)
WHERE rm.id IN $rids
RETURN ui {.*} AS ui, rm.id AS rid, a.role AS role
