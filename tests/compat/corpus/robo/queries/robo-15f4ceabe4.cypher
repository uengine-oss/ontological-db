MERGE (n:Event {id: $id})
SET n.name = $name, n.description = $description, n.version = 1, n.createdAt = datetime()
