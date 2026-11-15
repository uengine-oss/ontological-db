MERGE (evt:Event {id: $evt_id})
SET evt.name = $name,
    evt.version = 1,
    evt.description = $description,
    evt.createdAt = datetime()
WITH evt
OPTIONAL MATCH (cmd:Command {id: $cmd_id})
WHERE cmd IS NOT NULL
MERGE (cmd)-[:EMITS]->(evt)
RETURN evt.id as id
