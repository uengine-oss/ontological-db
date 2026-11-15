MATCH (n:Aggregate) WHERE n.name = $name OR n.title = $name RETURN n.id AS id LIMIT 1
