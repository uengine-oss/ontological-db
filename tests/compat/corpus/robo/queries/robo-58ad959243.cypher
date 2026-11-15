UNWIND $rows AS r
MATCH (us:UserStory {id: r.user_story_id})
MATCH (rm:ReadModel {id: r.readmodel_id})
MERGE (us)-[rel:IMPLEMENTS]->(rm)
  ON CREATE SET rel.confidence = coalesce(r.confidence, 0.9),
                rel.createdAt = datetime()
SET rel.confidence = coalesce(r.confidence, rel.confidence, 0.9)
RETURN {us_id: us.id, rm_id: rm.id} AS result
