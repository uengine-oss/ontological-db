MATCH (n {id: $node_id})
SET n.name = $name, n.description = $description, n.updatedAt = datetime()
RETURN n.id as id
