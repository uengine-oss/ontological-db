OPTIONAL MATCH (existing:Aggregate {key: $key})<-[:HAS_AGGREGATE]-(otherBC:BoundedContext)
WHERE otherBC.id <> $bc_id
RETURN otherBC.id as existing_bc
