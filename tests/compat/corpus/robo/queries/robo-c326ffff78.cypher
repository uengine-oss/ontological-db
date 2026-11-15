MATCH (c:Command)
WHERE NOT EXISTS { (:Policy)-[:INVOKES]->(c) }
RETURN c.id AS id,
       coalesce(c.displayName, c.name) AS displayName,
       c.name AS name
ORDER BY coalesce(c.displayName, c.name), c.id
