MATCH (rm:ReadModel {id: $readmodel_id})-[:HAS_CQRS]->(cqrs:CQRSConfig)
OPTIONAL MATCH (cqrs)-[:HAS_OPERATION]->(op:CQRSOperation)
OPTIONAL MATCH (op)-[:HAS_MAPPING]->(m:CQRSMapping)
OPTIONAL MATCH (op)-[:HAS_WHERE]->(w:CQRSWhere)
DETACH DELETE cqrs, op, m, w
RETURN count(cqrs) as deleted
