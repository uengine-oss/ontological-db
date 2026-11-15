MATCH (cmd:Command {id: $id})-[:EMITS]->(evt:Event)
RETURN evt.name as name
ORDER BY name
