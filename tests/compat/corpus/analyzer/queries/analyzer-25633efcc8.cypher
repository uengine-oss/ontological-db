MATCH (table:TABLE)-[:HAS_COLUMN]->(column:COLUMN)
WHERE table._owner = $owner AND column._owner = $owner
  AND ((table.name = $table_name OR table._id ENDS WITH $table_name) AND table.schema = $schema)
RETURN column.name AS name,
       table.name AS table_name,
       column.data_type AS data_type,
       coalesce(column.nullable, true) AS nullable,
       coalesce(column.logical_name, '') AS logical_name,
       coalesce(column.description, '') AS description,
       coalesce(column.summary, '') AS summary
ORDER BY column.name
