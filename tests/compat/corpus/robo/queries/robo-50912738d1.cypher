MATCH (bc:BoundedContext {id:$id}) RETURN coalesce(bc.version,0) AS v
