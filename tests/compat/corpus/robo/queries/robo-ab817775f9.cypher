MATCH (n:BoundedContext {id: $id})
RETURN n.id AS id, n.updatedAt AS updatedAt,
n.name AS name, n.description AS description
