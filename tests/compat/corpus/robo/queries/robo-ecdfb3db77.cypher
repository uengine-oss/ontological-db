MATCH (bc:BoundedContext) WHERE bc.name = $n OR bc.displayName = $n RETURN bc.id AS id LIMIT 1
