MATCH (e {id: $aid})
SET e.version = coalesce(e.version, 0) + 1
RETURN e.version AS v
