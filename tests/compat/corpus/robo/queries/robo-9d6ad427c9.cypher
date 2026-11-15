MATCH (cmd:Command {id: $source_id})
MATCH (evt:Event {id: $target_id})
MERGE (cmd)-[:EMITS]->(evt)
RETURN cmd.id as id
