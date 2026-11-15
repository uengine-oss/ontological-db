MATCH (us:UserStory {id: $user_story_id})
SET us.role = $role,
    us.updatedAt = datetime()
RETURN us {.id, .role, .action, .benefit, .priority, .status, uiDescription: us.uiDescription} as user_story
