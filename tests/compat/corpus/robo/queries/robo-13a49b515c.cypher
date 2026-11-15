MATCH (us:UserStory)
OPTIONAL MATCH (us)-[:IMPLEMENTS]->(bc:BoundedContext)
OPTIONAL MATCH (f:Feature)-[:HAS_USER_STORY]->(us)
OPTIONAL MATCH (us)-[:IMPLEMENTS]->(cmd:Command)
RETURN us {.id, .role, .action, .benefit, .priority, .status, .acceptanceCriteria} AS us,
       bc.id AS bcId,
       f.id AS featureId,
       cmd.id AS cmdId, cmd.name AS cmdName
