MATCH (op:CQRSOperation {id: $operation_id})
OPTIONAL MATCH (op)-[:HAS_MAPPING]->(m:CQRSMapping)
OPTIONAL MATCH (op)-[:HAS_WHERE]->(w:CQRSWhere)
DETACH DELETE op, m, w
RETURN count(op) as deleted
