MATCH (us:UserStory {id: $user_story_id})
MATCH (bc:BoundedContext {id: $bc_id})
MERGE (us)-[r:IMPLEMENTS]->(bc)
SET r.confidence = $confidence,
    r.createdAt = datetime()
RETURN us.id as us_id, bc.id as bc_id
