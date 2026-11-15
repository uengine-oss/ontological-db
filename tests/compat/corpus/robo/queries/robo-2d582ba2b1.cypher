MATCH (bc:BoundedContext {id:$id}) RETURN coalesce(bc.displayName,bc.name) AS name, bc.description AS d
