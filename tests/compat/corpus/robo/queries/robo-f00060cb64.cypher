MATCH (bc:BoundedContext)-[:HAS_EVENT]->(evt:Event {id: $evt_id}) RETURN bc.id AS bcId, bc.name AS bcName LIMIT 1
