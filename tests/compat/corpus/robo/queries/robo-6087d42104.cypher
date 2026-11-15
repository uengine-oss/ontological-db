MATCH (us:UserStory)
WHERE NOT (us)-[:IMPLEMENTS]->(:BoundedContext)
RETURN us.id as id, us.role as role, us.action as action
