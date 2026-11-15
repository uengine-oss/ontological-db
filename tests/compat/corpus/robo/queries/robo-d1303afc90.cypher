MATCH (us:UserStory {id: $user_story_id})
MATCH (pol:Policy {id: $policy_id})
MERGE (us)-[r:IMPLEMENTS]->(pol)
SET r.confidence = $confidence,
    r.createdAt = datetime()
RETURN us.id, pol.id
