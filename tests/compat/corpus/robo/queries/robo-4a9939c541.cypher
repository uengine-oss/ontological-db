MATCH (rm:ReadModel {id: $readmodel_id})
MERGE (cqrs:CQRSConfig {id: $cqrs_id})
SET cqrs.readmodelId = $readmodel_id
MERGE (rm)-[:HAS_CQRS]->(cqrs)
WITH cqrs
MATCH (evt:Event {id: $trigger_event_id})
MERGE (op:CQRSOperation {id: $op_id})
SET op.operationType = $operation_type,
    op.cqrsConfigId = $cqrs_id,
    op.triggerEventId = $trigger_event_id
MERGE (cqrs)-[:HAS_OPERATION]->(op)
MERGE (op)-[:TRIGGERED_BY]->(evt)
RETURN op {.id, .operationType, .cqrsConfigId, .triggerEventId} as operation
