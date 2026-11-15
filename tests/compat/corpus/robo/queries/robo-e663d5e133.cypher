MATCH (n {id: $target_id})
SET n.description = $description, n.updatedAt = datetime()
RETURN n.id as id
