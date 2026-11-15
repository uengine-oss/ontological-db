MERGE (n:Command {id: $id})
SET n.name = $name, n.description = $description, n.createdAt = datetime()
