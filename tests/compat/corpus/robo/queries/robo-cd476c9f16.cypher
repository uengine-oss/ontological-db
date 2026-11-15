MATCH (rm:ReadModel)-[:HAS_CQRS]->(:CQRSConfig)
      -[:HAS_OPERATION]->(:CQRSOperation)-[:TRIGGERED_BY]->(evt:Event)
WHERE evt.id IN $eids
RETURN rm {.*} AS rm, evt.id AS eid
