MATCH (bc:BoundedContext)-[:HAS_UI]->(:UI {id: $node_id}) RETURN bc LIMIT 1
