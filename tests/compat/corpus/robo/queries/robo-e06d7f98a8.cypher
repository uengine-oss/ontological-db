MATCH (bc:BoundedContext {id: $context_id})-[:HAS_READMODEL]->(rm:ReadModel)-[:HAS_CQRS]->(cqrs:CQRSConfig)-[:HAS_OPERATION]->(op:CQRSOperation)
OPTIONAL MATCH (op)-[:TRIGGERED_BY]->(evt:Event)
RETURN rm.id as readmodelId,
       op {.id, .operationType, .triggerEventId} as operation,
       evt.name as triggerEventName
ORDER BY rm.id, op.operationType
