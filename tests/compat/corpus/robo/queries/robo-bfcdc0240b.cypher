MATCH (bc:BoundedContext)-[:HAS_AGGREGATE]->(:Aggregate)-[:HAS_COMMAND]->(:Command {id: $node_id}) RETURN bc LIMIT 1
