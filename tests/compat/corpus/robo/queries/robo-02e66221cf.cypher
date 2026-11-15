MATCH (bc:BoundedContext)-[:HAS_AGGREGATE]->(:Aggregate)-[:HAS_COMMAND]->(:Command)-[:EMITS]->(:Event {id: $node_id}) RETURN bc LIMIT 1
