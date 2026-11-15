MATCH (table:TABLE)
WHERE table._owner = $owner AND table.schema = $schema
OPTIONAL MATCH (table)-[:HAS_COLUMN]->(column:COLUMN {_owner: $owner})
WITH table, count(column) AS column_count
RETURN table.name AS name,
       table.schema AS schema,
       table.datasource AS datasource,
       table.logical_name AS logical_name,
       table.description AS description,
       table.summary AS summary,
       column_count
ORDER BY table.datasource, table.schema, table.name
LIMIT $limit
