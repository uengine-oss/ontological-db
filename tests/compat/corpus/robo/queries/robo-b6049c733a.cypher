MATCH (rm:ReadModel {id: $rid})-[:HAS_CQRS]->(:CQRSConfig)
      -[:HAS_OPERATION]->(:CQRSOperation)-[:TRIGGERED_BY]->(:Event)
      <-[:EMITS]-(:Command)<-[a:ATTACHED_TO]-(u:UI)
WHERE a.role IS NULL OR a.role <> 'display'
RETURN u.id AS uid LIMIT 1
