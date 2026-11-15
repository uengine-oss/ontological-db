MATCH (ui:UI {id: $ui_id})
MATCH (target:Command {id: $target_id})
MERGE (ui)-[:ATTACHED_TO]->(target)
