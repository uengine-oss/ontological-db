MATCH (c:Command {id: $cid})
RETURN coalesce(c.displayName, c.name) AS displayName
