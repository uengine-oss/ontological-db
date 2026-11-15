MATCH (n)
WHERE n:UserStory OR n:BoundedContext OR n:Aggregate OR n:Feature
RETURN n.id AS id,
       labels(n)[0] AS label,
       COALESCE(n.title, n.name, n.action, n.key, '') AS name
LIMIT $limit
