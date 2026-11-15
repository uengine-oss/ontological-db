MATCH (op:CQRSOperation)-[:TRIGGERED_BY]->(evt:Event {id: $evtId})
WHERE op.cqrsConfigId = $cqrsId
DETACH DELETE op
