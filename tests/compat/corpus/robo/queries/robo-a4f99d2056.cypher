MATCH (bc:BoundedContext)
RETURN bc.id as id, bc.name as name, bc.description as description
ORDER BY bc.name
