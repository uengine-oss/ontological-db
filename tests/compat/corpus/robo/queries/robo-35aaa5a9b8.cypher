MERGE (n:Event {id: $target_id})
SET n.name = $name, n.description = $description, n.version = 1, n.createdAt = datetime()
WITH n
MATCH (cmd:Command {id: $cmd_id})
MERGE (cmd)-[:EMITS]->(n)
RETURN n.id as id
