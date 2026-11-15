MATCH (f:Feature {id: $id})-[:HAS_USER_STORY]->(us:UserStory)
DETACH DELETE us
