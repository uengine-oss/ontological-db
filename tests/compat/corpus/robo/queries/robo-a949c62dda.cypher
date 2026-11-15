MATCH (us:UserStory {id: $user_story_id})
MATCH (agg:Aggregate {id: $aggregate_id})
MERGE (us)-[r:IMPLEMENTS]->(agg)
SET r.confidence = $confidence,
    r.createdAt = datetime()
RETURN us.id, agg.id
