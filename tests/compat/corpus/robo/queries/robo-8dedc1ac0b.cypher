MATCH (bc:BoundedContext) WHERE bc.name CONTAINS $name OR bc.key CONTAINS $name RETURN bc.id LIMIT 1
