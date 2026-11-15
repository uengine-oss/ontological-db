MATCH (rm:ReadModel {id: $readmodel_id})
MERGE (cqrs:CQRSConfig {id: $cqrs_id})
SET cqrs.readmodelId = $readmodel_id
MERGE (rm)-[:HAS_CQRS]->(cqrs)
RETURN cqrs {.id, .readmodelId} as config
