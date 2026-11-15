MATCH (us:UserStory {id: $user_story_id})
SET us.role = $role,
    us.action = $action,
    us.benefit = $benefit,
    us.updatedAt = datetime()
RETURN us.id as id
