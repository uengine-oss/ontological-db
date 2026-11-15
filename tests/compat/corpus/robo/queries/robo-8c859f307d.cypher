UNWIND $rows AS r
MATCH (ui:UI {id: r.ui_id})
MATCH (target:Command {id: r.target_id})
MERGE (ui)-[:ATTACHED_TO]->(target)
RETURN ui.id AS ui_id
