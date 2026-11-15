MATCH (us:UserStory)-[:IMPLEMENTS]->(bc:BoundedContext)
WITH bc, collect(us {.id, .role, .action, .benefit}) AS stories
RETURN bc.id AS bcId, bc.key AS bcKey, bc.name AS bcName, stories
ORDER BY bc.name
