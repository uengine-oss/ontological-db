UNWIND $rows AS r
MATCH (us:UserStory {id: r.user_story_id})
MATCH (agg:Aggregate {id: r.aggregate_id})
MERGE (us)-[rel:IMPLEMENTS]->(agg)
  ON CREATE SET rel.confidence = coalesce(r.confidence, 0.9),
                rel.createdAt = datetime()
SET rel.confidence = coalesce(r.confidence, rel.confidence, 0.9)
RETURN {us_id: us.id, agg_id: agg.id} AS result
