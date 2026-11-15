MATCH (bc:BoundedContext) WHERE bc.id IN $ids RETURN bc.id AS id, bc.key AS key
