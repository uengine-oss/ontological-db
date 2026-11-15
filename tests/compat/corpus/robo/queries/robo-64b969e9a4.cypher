MATCH (bc:BoundedContext {id: $bc_id})
RETURN bc.id as id, bc.name as name
