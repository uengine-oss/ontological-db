MATCH (n {id: $node_id})
OPTIONAL MATCH (bc:BoundedContext)
WHERE (bc)-[:HAS_AGGREGATE]->(n)
   OR (bc)-[:HAS_AGGREGATE]->(:Aggregate)-[:HAS_COMMAND]->(n)
   OR (bc)-[:HAS_AGGREGATE]->(:Aggregate)-[:HAS_COMMAND]->(:Command)-[:EMITS]->(n)
   OR (bc)-[:HAS_POLICY]->(n)
   OR (bc)-[:HAS_UI]->(n)
   OR (bc)-[:HAS_READMODEL]->(n)
   OR (bc)-[:HAS_EVENT]->(n)
RETURN bc {.id, .name, .description} as boundedContext
LIMIT 1
