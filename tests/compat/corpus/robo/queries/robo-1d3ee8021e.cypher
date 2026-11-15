MATCH (f:Feature {id: $id})-[:HAS_USER_STORY]->(us:UserStory)
RETURN us.role AS role, us.action AS action, us.benefit AS benefit
