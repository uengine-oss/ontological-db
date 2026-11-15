MATCH (f:Feature) WHERE f.id <> $id RETURN f.name AS s LIMIT 40
