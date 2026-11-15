MATCH (bc:BoundedContext)-[:HAS_FEATURE]->(f:Feature)
RETURN f.id AS id, f.name AS name, f.description AS description,
       f.source AS source, bc.id AS bcId, f.sequence AS sequence,
       f.edgeCases AS edgeCases, f.assumptions AS assumptions
ORDER BY f.sequence, f.name
