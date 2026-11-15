MATCH (cmd:Command {id: $node_id})-[:EMITS]->(evt:Event)
RETURN evt
