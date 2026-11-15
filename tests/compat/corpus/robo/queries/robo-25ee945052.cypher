MATCH (f:Feature) WHERE f.name CONTAINS $name RETURN f.id LIMIT 1
