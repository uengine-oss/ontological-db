MATCH (op:CQRSOperation {id: $operation_id})
MATCH (tgtProp:Property {id: $target_property_id})
MATCH (srcProp:Property {id: $source_event_field_id})
MERGE (w:CQRSWhere {id: $where_id})
SET w.operationId = $operation_id,
    w.operator = $operator
MERGE (op)-[:HAS_WHERE]->(w)
MERGE (w)-[:TARGET]->(tgtProp)
MERGE (w)-[:SOURCE_EVENT_FIELD]->(srcProp)
RETURN w {.id, .operationId, .operator} as whereCondition
