MATCH (u:UI)-[:ATTACHED_TO]->(h:Command {id: $h}) WHERE u.id <> $u RETURN u.id AS id LIMIT 1
