MATCH (t:TABLE {name: $table_name})
WHERE t._owner = $owner
  AND t.datasource = $datasource
  AND (t.schema = $schema_name
       OR ($schema_name = 'public' AND coalesce(t.schema, '') = ''))
  AND (t.description IS NULL OR t.description = '' OR t.description = 'N/A')
SET t.description = $description
RETURN count(t) AS updated
