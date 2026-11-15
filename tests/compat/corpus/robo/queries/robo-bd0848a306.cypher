MATCH (cmd:Command {id: $command_id}), (evt:Event {id: $evt_id})
MERGE (cmd)-[r:EMITS]->(evt)
ON CREATE SET r.isGuaranteed = true
SET r.isGuaranteed = coalesce(r.isGuaranteed, true)
