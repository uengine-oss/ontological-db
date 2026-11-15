MATCH (us:UserStory {id: $id})
WITH us, coalesce(us.clarifications, '[]') AS raw
WITH us, CASE WHEN raw = '' THEN '[]' ELSE raw END AS raw
RETURN raw
