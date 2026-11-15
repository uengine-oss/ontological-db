MATCH (bc:BoundedContext)-[:HAS_EVENT]->(evt:Event)
WHERE bc.id <> $bc_id
RETURN evt.name AS name, bc.name AS bcName
ORDER BY bc.name, evt.sequence
