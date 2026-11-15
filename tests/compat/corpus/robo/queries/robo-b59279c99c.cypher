MATCH (bc:BoundedContext {id: $context_id})-[:HAS_AGGREGATE]->(agg:Aggregate)-[:HAS_COMMAND]->(cmd:Command)-[:EMITS]->(evt:Event)
RETURN agg.id as aggregateId, cmd.id as commandId, evt {.id, .name, .displayName, .version, .payload} as event
ORDER BY evt.name
