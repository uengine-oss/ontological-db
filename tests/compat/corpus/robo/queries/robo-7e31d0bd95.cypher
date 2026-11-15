MATCH (bc:BoundedContext {id: $context_id})-[:HAS_READMODEL]->(rm:ReadModel)
RETURN rm {.id, .name} as readmodel
    ORDER BY readmodel.name
