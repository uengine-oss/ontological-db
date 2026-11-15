MATCH (us:UserStory {id: $user_story_id})
OPTIONAL MATCH (us)-[r:CHANGED_TO]->(version)
RETURN us {.*} as current,
       collect(version {.*, changedAt: r.changedAt}) as history
ORDER BY r.changedAt DESC
