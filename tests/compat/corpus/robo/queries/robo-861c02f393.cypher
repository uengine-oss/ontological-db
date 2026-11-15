MATCH (bc:BoundedContext {id: $id})-[:HAS_READMODEL]->(rm:ReadModel)
RETURN rm.id AS id, rm.name AS name, rm.description AS description
ORDER BY rm.name
