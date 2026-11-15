MATCH (us:UserStory {id: $us_id})
MATCH (f:Feature {id: $feature_id})
OPTIONAL MATCH (us)<-[old:HAS_USER_STORY]-(:Feature)
DELETE old
MERGE (f)-[r:HAS_USER_STORY]->(us)
  ON CREATE SET r.createdAt = datetime()
SET r.source = $source, r.confidence = $confidence
RETURN us.id AS us_id
