MATCH (table:TABLE)
WHERE table._owner = $owner
  AND table.description IS NOT NULL
  AND table.description <> ''
RETURN table.name AS name,
       table.schema AS schema,
       table.description AS description
ORDER BY table.name
LIMIT 200
