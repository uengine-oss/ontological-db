MATCH (us:UserStory)
WHERE NOT (us)-[:IMPLEMENTS]->(:Aggregate)
  AND NOT (us)-[:IMPLEMENTS]->(:Command)
  AND NOT (us)-[:IMPLEMENTS]->(:Event)
  AND NOT (us)-[:IMPLEMENTS]->(:Policy)
  AND NOT (us)-[:IMPLEMENTS]->(:ReadModel)
  AND (us)-[:IMPLEMENTS]->(:BoundedContext {id: $scopeId})
OPTIONAL MATCH (us)-[:IMPLEMENTS]->(bc:BoundedContext)
OPTIONAL MATCH (f:Feature)-[:HAS_USER_STORY]->(us)
RETURN us.id AS id, us.role AS role, us.action AS action, us.benefit AS benefit,
       bc.id AS bcId, f.id AS featureId
ORDER BY us.id
