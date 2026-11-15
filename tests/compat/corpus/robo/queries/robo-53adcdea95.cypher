MERGE (n:Event {id: $target_id})
SET n.name = $name, n.description = $description, n.version = 1, n.createdAt = datetime()
RETURN n.id as id
