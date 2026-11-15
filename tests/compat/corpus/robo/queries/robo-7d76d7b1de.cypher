MATCH (bc:BoundedContext {id: $bc_id})
SET bc.classification = $classification,
    bc.version = coalesce(bc.version, 0) + 1
RETURN bc.classification AS classification, bc.version AS version
