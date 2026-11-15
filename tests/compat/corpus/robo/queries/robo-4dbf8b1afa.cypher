MATCH (n {id: $node_id})
SET n.name = $name, n.updatedAt = datetime()
