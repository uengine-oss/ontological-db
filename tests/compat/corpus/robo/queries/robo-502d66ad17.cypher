MATCH (rm:ReadModel {id: $rm_id})
MATCH (evt:Event {id: $evt_id})
MERGE (cqrs:CQRSConfig {id: $cqrs_id})
SET cqrs.readmodelId = rm.id
MERGE (rm)-[:HAS_CQRS]->(cqrs)
MERGE (op:CQRSOperation {id: $op_id})
SET op.operationType = 'INSERT',
    op.cqrsConfigId = $cqrs_id,
    op.triggerEventId = evt.id
MERGE (cqrs)-[:HAS_OPERATION]->(op)
MERGE (op)-[:TRIGGERED_BY]->(evt)
RETURN evt.id AS eventId
