MATCH (bc:BoundedContext {id: $id})
OPTIONAL MATCH (bc)-[:HAS_FEATURE]->(f:Feature)
OPTIONAL MATCH (f)-[:HAS_USER_STORY]->(us:UserStory)
WITH collect(DISTINCT bc.id) + collect(DISTINCT f.id)
   + collect(DISTINCT us.id) AS ids
RETURN [x IN ids WHERE x IS NOT NULL] AS ids
