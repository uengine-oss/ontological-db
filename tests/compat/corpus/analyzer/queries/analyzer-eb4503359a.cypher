MATCH (t:TABLE {name: $table_name})
  -[:HAS_COLUMN]->(c:COLUMN {name: $col_name})
WHERE t._owner = $owner AND c._owner = $owner
  AND t.datasource = $datasource
  AND (t.schema = $schema_name
       OR ($schema_name = 'public' AND coalesce(t.schema, '') = ''))
  AND (c.description IS NULL OR c.description = '' OR c.description = 'N/A')
SET c.description = $description
RETURN count(c) AS updated
