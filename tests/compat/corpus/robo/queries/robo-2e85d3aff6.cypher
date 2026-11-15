MATCH (bc:BoundedContext {id: $context_id})-[:HAS_AGGREGATE]->(agg:Aggregate)
RETURN agg {.id, .name, .displayName, .rootEntity, .invariants, .enumerations, .valueObjects, .exceptions} as aggregate
ORDER BY agg.name
