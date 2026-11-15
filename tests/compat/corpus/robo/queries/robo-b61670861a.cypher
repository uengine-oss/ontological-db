MATCH (ui:UI {id: $uiId}), (target {id: $targetId})
MERGE (ui)-[:ATTACHED_TO]->(target)
