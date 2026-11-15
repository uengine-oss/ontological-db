MATCH (n {id: $id})
SET n.deleted = true, n.deletedAt = datetime()
RETURN n.id as id
