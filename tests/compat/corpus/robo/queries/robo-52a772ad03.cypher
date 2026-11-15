MERGE (n:Policy {id: $target_id})
SET n.name = $name, n.description = $description, n.createdAt = datetime()
RETURN n.id as id
