MATCH (u:UI {id: $uid}), (rm:ReadModel {id: $rid})
MERGE (u)-[r:ATTACHED_TO]->(rm)
SET r.role = 'display'
