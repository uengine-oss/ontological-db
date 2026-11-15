MATCH (bc:BoundedContext {id: $bc_id})
WITH bc, coalesce(bc.classification, null) as old
SET bc.classification = $cls,
    bc.version = coalesce(bc.version, 0) + 1
RETURN bc.classification as cls, bc.version as version, old
