MATCH (us:UserStory) WHERE us.id <> $id
RETURN us.id AS id, us.role AS role, us.action AS action
