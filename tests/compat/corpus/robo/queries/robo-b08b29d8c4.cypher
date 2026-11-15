MATCH (us:UserStory {id: $id})-[:IMPLEMENTS]->(rm:ReadModel)
OPTIONAL MATCH (rm)-[:HAS_CQRS]->(:CQRSConfig)
      -[:HAS_OPERATION]->(:CQRSOperation)-[:TRIGGERED_BY]->(:Event)<-[:EMITS]-(cmd:Command)
RETURN collect(DISTINCT rm.id) AS rm_ids, collect(DISTINCT cmd.id) AS cmd_ids
