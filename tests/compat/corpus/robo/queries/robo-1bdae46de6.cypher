MATCH (us:UserStory)-[:IMPLEMENTS]->(bc:BoundedContext {id: $context_id})
RETURN us {.id, .role, .action, .benefit, .priority, .status} as userStory
ORDER BY us.id
