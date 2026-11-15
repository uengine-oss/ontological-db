MATCH (op:CQRSOperation {id: $operation_id})
MATCH (srcProp:Property {id: $source_property_id})
MATCH (tgtProp:Property {id: $target_property_id})
MERGE (m:CQRSMapping {id: $mapping_id})
SET m.operationId = $operation_id,
    m.sourceType = $source_type,
    m.staticValue = null
MERGE (op)-[:HAS_MAPPING]->(m)
MERGE (m)-[:SOURCE]->(srcProp)
MERGE (m)-[:TARGET]->(tgtProp)
RETURN m {.id, .operationId, .sourceType} as mapping
