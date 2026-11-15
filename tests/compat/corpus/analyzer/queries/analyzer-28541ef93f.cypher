MATCH (source:TABLE)-[relationship:FK]->(target:TABLE)
WHERE source._owner = $owner AND target._owner = $owner
  AND relationship._owner IN $relationship_owners
RETURN source.name AS from_table,
       source.schema AS from_schema,
       relationship.from_column AS from_column,
       target.name AS to_table,
       target.schema AS to_schema,
       relationship.to_column AS to_column,
       'FK' AS relationship_type,
       coalesce(relationship.description, '') AS description
ORDER BY source.name, target.name, from_column, to_column
