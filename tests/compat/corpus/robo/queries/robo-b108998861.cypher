MATCH (n {id: $node_id})
SET n.name = $name, n.updatedAt = datetime()
RETURN n.id as id
