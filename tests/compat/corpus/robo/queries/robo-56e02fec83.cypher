UNWIND $rows AS r
OPTIONAL MATCH (existing:Aggregate {key: r.key})<-[:HAS_AGGREGATE]-(otherBC:BoundedContext)
WHERE otherBC.id <> r.bc_id
WITH r, otherBC
WHERE otherBC IS NOT NULL
RETURN r.key AS key, otherBC.id AS existing_bc
