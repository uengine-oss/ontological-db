MATCH (cqrs:CQRSConfig {id: $cqrsId}), (evt:Event {id: $evtId})
MERGE (op:CQRSOperation {id: $opId})
  ON CREATE SET op.operationType = 'INSERT',
               op.cqrsConfigId = $cqrsId,
               op.triggerEventId = $evtId
MERGE (cqrs)-[:HAS_OPERATION]->(op)
MERGE (op)-[:TRIGGERED_BY]->(evt)
