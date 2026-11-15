MATCH (us:UserStory {id: $user_story_id})
MATCH (cmd:Command {id: $command_id})
MERGE (us)-[r:IMPLEMENTS]->(cmd)
SET r.confidence = $confidence,
    r.createdAt = datetime()
RETURN us.id, cmd.id
