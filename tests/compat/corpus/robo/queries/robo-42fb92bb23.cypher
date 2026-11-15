UNWIND $ids AS id
MATCH (us:UserStory {id: id})
RETURN us.id AS userStoryId, coalesce(us.clarifications, '[]') AS raw
