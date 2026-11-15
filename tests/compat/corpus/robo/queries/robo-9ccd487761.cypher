MATCH (u:UI {id: $u})-[:ATTACHED_TO]->(t) WHERE t.id <> $h RETURN labels(t)[0] AS lbl LIMIT 1
