MATCH (bc:BoundedContext)-[:HAS_READMODEL]->(:ReadModel {id: $node_id}) RETURN bc LIMIT 1
