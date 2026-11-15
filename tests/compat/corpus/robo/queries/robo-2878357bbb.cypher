UNWIND $rows AS r
MATCH (us:UserStory {id: r.user_story_id})
MATCH (cmd:Command {id: r.command_id})
MERGE (us)-[rel:IMPLEMENTS]->(cmd)
  ON CREATE SET rel.confidence = coalesce(r.confidence, 0.9),
                rel.createdAt = datetime()
SET rel.confidence = coalesce(r.confidence, rel.confidence, 0.9)
RETURN {us_id: us.id, cmd_id: cmd.id} AS result
