MATCH (us:UserStory)-[]->(n {id: $id})
RETURN DISTINCT us.id AS id, us.role AS role, us.action AS action, us.sourceUnitId AS src
