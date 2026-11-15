MATCH (table:TABLE)
WHERE table._owner = $owner AND table.datasource IN $datasources
  AND ($schema IS NULL OR table.schema = $schema)
OPTIONAL MATCH (table)-[:HAS_COLUMN]->(column:COLUMN {_owner: $owner})
WITH table, column ORDER BY column.name
RETURN table.datasource AS datasource, table.schema AS schema, table.name AS name,
       table.table_type AS table_type,
       [c IN collect({name: column.name, data_type: column.data_type, nullable: column.nullable,
                      primary_key: column.primary_key, unique: column.unique}) WHERE c.name IS NOT NULL] AS columns
ORDER BY datasource, schema, name
