MATCH (bc:BoundedContext)-[:HAS_AGGREGATE]->(:Aggregate {id: $node_id}) RETURN bc LIMIT 1
