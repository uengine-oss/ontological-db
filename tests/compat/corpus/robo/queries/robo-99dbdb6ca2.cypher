MATCH (us:UserStory {id: $id})
RETURN coalesce(us.clarifications, '[]') AS raw
