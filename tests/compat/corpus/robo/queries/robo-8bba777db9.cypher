MATCH (bc:BoundedContext {id: $bc_id})-[:HAS_EVENT]->(evt:Event) RETURN evt.id AS id, evt.name AS name, evt.displayName AS displayName
