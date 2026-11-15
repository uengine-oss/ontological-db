MATCH (table:TABLE)-[:HAS_COLUMN]->(column:COLUMN)
WHERE table.name = $table_name
  AND table._owner = $owner AND column._owner = $owner
  AND ($table_schema = '' OR table.schema = $table_schema)
  AND column.name = $column_name
SET column.description = $description
RETURN column.name AS name
