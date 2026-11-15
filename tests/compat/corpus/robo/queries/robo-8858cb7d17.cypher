MATCH (us:UserStory {id: $usid})
MATCH (rm:ReadModel {id: $rmid})
MERGE (us)-[r1:IMPLEMENTS]->(rm)
  ON CREATE SET r1.confidence = $conf
MERGE (rm)-[r2:IMPLEMENTS]->(us)
  ON CREATE SET r2.confidence = $conf
RETURN id(us) AS uid
