MATCH (ui:UI {id: $ui_id})
MATCH (target:Command {id: $attached_to_id})
MERGE (ui)-[:ATTACHED_TO]->(target)
