MATCH (table:TABLE)
WHERE table._owner = $owner
  AND table.datasource = $datasource
  AND (table.description IS NULL OR table.description = '')
OPTIONAL MATCH (table)-[:HAS_COLUMN]->(column:COLUMN {_owner: $owner})
RETURN table.name AS table_name,
       table.schema AS schema_name,
       collect({
         name: column.name,
         data_type: column.data_type,
         description: column.description
       }) AS columns
