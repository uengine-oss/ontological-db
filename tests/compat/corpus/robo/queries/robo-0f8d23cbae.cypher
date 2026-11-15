MATCH (f:Feature {id: $id})-[:HAS_USER_STORY]->(us:UserStory)
RETURN us.id AS id
