MATCH (bc:BoundedContext {id: $bc_id})
RETURN bc.classification AS old, coalesce(bc.version, 0) AS version
