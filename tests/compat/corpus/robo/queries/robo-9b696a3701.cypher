MATCH (us:UserStory {id: $user_story_id})
MATCH (evt:Event {id: $event_id})
MERGE (us)-[r:IMPLEMENTS]->(evt)
SET r.confidence = $confidence,
    r.createdAt = datetime()
RETURN us.id, evt.id
