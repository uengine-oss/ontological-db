MATCH (bc:BoundedContext {id:$id})-[:HAS_FEATURE]->(f:Feature) RETURN f.id AS id, f.name AS name
