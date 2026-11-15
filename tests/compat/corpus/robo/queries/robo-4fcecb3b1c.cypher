MATCH (bc:BoundedContext {id: $context_id})-[:HAS_AGGREGATE]->(agg:Aggregate)-[:HAS_COMMAND]->(cmd:Command)
RETURN agg.id as aggregateId, cmd {.id, .name, .displayName, .actor, .category, .inputSchema} as command
ORDER BY cmd.name
