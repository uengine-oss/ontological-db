MATCH (bc:BoundedContext {id: $context_id})-[:HAS_READMODEL]->(rm:ReadModel)
RETURN rm {.id, .name, .displayName, .description, .provisioningType, .actor, .isMultipleResult} as readmodel
ORDER BY rm.name
