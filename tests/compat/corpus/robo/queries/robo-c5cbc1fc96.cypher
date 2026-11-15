MATCH (n {id: $id})
SET n.name = $name, n.description = $description
RETURN n.id as id
