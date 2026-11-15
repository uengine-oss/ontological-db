MATCH (us:UserStory {id: $id})
OPTIONAL MATCH (us)-[:IMPLEMENTS]->(cmd:Command)
RETURN us {.id, .role, .action, .benefit, .priority, .status, .acceptanceCriteria} AS us,
       cmd.id AS cmdId, cmd.name AS cmdName
