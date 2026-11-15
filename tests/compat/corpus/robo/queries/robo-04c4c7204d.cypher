MATCH (rm:ReadModel {id: $rmId})
MERGE (cqrs:CQRSConfig {id: $cqrsId})
  ON CREATE SET cqrs.readmodelId = $rmId
MERGE (rm)-[:HAS_CQRS]->(cqrs)
