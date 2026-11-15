MATCH (bc:BoundedContext)-[:HAS_FEATURE]->(f:Feature)
RETURN f.id AS id, f.name AS name, bc.id AS bcId
