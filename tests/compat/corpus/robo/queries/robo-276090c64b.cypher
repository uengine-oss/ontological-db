MATCH (evt:Event {id: $id})
MATCH (cmd:Command {id: $cmd_id})
MERGE (cmd)-[:EMITS {isGuaranteed: true}]->(evt)
