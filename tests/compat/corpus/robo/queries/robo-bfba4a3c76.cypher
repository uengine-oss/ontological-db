MATCH (u:UI {id: $uid})
OPTIONAL MATCH (bc:BoundedContext)-[:HAS_UI]->(u)
RETURN u.id AS id,
       coalesce(u.displayName, u.name) AS displayName,
       u.name AS name,
       u.description AS description,
       bc.name AS bcName,
       bc.description AS bcDescription
LIMIT 1
