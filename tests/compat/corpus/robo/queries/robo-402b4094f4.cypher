MATCH (bc:BoundedContext)-[:HAS_POLICY]->(:Policy {id: $node_id}) RETURN bc LIMIT 1
