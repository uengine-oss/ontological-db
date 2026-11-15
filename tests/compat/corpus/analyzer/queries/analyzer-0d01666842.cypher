MATCH (source:TABLE)-[relationship:FK]->(target:TABLE)
WHERE source._owner = $owner AND target._owner = $owner
  AND source.datasource IN $datasources AND target.datasource IN $datasources
  AND NOT (relationship._owner = $catalog_owner AND relationship.description = $inferred)
RETURN source.datasource AS from_datasource, source.schema AS from_schema, source.name AS from_table,
       relationship.from_column AS from_column,
       target.datasource AS to_datasource, target.schema AS to_schema, target.name AS to_table,
       relationship.to_column AS to_column
