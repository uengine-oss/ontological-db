MATCH (rm:ReadModel {id: $readmodel_id})
OPTIONAL MATCH (rm)-[:HAS_CQRS]->(cqrs:CQRSConfig)
OPTIONAL MATCH (cqrs)-[:HAS_OPERATION]->(op:CQRSOperation)
OPTIONAL MATCH (op)-[:TRIGGERED_BY]->(evt:Event)
OPTIONAL MATCH (op)-[:HAS_MAPPING]->(m:CQRSMapping)
OPTIONAL MATCH (m)-[:SOURCE]->(srcProp:Property)
OPTIONAL MATCH (m)-[:TARGET]->(tgtProp:Property)
OPTIONAL MATCH (op)-[:HAS_WHERE]->(w:CQRSWhere)
OPTIONAL MATCH (w)-[:TARGET]->(whereTgtProp:Property)
OPTIONAL MATCH (w)-[:SOURCE_EVENT_FIELD]->(whereSrcProp:Property)
WITH cqrs, op, evt,
     collect(DISTINCT {
         id: m.id,
         sourceType: m.sourceType,
         staticValue: m.staticValue,
         sourcePropertyId: srcProp.id,
         sourcePropertyName: srcProp.name,
         targetPropertyId: tgtProp.id,
         targetPropertyName: tgtProp.name
     }) as mappings,
     collect(DISTINCT {
         id: w.id,
         operator: w.operator,
         targetPropertyId: whereTgtProp.id,
         targetPropertyName: whereTgtProp.name,
         sourceEventFieldId: whereSrcProp.id,
         sourceEventFieldName: whereSrcProp.name
     }) as whereConditions
WITH cqrs, collect(DISTINCT {
    id: op.id,
    operationType: op.operationType,
    triggerEventId: evt.id,
    triggerEventName: evt.name,
    mappings: mappings,
    whereConditions: whereConditions
}) as operations
RETURN {
    id: cqrs.id,
    readmodelId: coalesce(cqrs.readmodelId, $readmodel_id),
    operations: operations
} as config
