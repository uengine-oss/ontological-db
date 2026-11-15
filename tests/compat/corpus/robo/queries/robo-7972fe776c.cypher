MATCH (n:BoundedContext {id: $id}) SET n.name = $name RETURN n.updatedAt AS updatedAt
