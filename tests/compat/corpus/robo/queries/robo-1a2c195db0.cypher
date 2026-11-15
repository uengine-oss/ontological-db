MATCH (n {id: $node_id})
SET n.name = $new_name, n.updatedAt = datetime()
RETURN n.id as id
