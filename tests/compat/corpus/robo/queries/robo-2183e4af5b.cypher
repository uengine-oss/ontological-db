MATCH (bc:BoundedContext {id: $bc_id})-[:HAS_FEATURE|CONTAINS]->(f:Feature) WHERE f.name CONTAINS $name RETURN f.id LIMIT 1
