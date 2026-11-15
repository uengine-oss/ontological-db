MATCH (table:TABLE)
WHERE table.name = $table_name AND table._owner = $owner
  AND ($schema = '' OR table.schema = $schema OR table.schema IS NULL)
SET table.description = $description
RETURN table.name AS name
