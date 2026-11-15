MATCH (us:UserStory) WHERE us.id <> $id
RETURN coalesce(us.role,'') + ': ' + coalesce(us.action,'') AS s
LIMIT 60
