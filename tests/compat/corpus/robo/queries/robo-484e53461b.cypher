MATCH (bc:BoundedContext {id: $id})-[:HAS_FEATURE]->(:Feature)
      -[:HAS_USER_STORY]->(us:UserStory)
RETURN DISTINCT us.id AS id
