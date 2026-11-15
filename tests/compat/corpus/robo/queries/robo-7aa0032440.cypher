MATCH (rm:ReadModel {id: $rid})-[:HAS_CQRS]->(:CQRSConfig)-[:HAS_OPERATION]->(:CQRSOperation)-[:TRIGGERED_BY]->(evt:Event) RETURN evt.sequence AS seq LIMIT 1
