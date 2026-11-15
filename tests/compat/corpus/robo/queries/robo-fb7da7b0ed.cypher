MATCH (bc:BoundedContext {id: $bc_id})-[:HAS_EVENT]->(evt:Event)
RETURN evt.name AS name, evt.description AS description
ORDER BY evt.sequence
