MATCH (n:Event {id: $id})-[:IMPLEMENTS]->(us:UserStory)
RETURN DISTINCT us.id AS id, us.role AS role, us.action AS action,
       coalesce(us.id, us.sourceUnitId) AS src
UNION
MATCH (us:UserStory)-[:HAS_EVENT]->(n:Event {id: $id})
RETURN DISTINCT us.id AS id, us.role AS role, us.action AS action,
       coalesce(us.id, us.sourceUnitId) AS src
