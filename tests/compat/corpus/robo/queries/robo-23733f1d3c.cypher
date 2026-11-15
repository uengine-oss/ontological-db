MATCH (us:UserStory {id: $us_id})
SET us.role = $role,
    us.action = $action,
    us.benefit = $benefit,
    us.updatedAt = datetime()
