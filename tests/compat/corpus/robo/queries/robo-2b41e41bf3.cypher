MERGE (n:Policy {id: $id})
SET n.name = $name, n.description = $description, n.createdAt = datetime()
