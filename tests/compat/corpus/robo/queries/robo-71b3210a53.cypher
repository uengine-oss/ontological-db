MATCH (f:Feature {id: $id})
OPTIONAL MATCH (f)-[:HAS_USER_STORY]->(us:UserStory)
WITH collect(DISTINCT f.id) + collect(DISTINCT us.id) AS ids
RETURN [x IN ids WHERE x IS NOT NULL] AS ids
