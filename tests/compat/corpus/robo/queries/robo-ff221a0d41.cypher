MATCH (n {id: $target_id})
SET n.name = $new_name, n.updatedAt = datetime()
RETURN n.id as id
