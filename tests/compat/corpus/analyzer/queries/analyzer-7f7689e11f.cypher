MATCH (source:TABLE {name: $from_table})
MATCH (target:TABLE {name: $to_table})
WHERE source._owner = $owner AND target._owner = $owner
  AND ($from_schema = '' OR source.schema = $from_schema)
  AND ($to_schema = '' OR target.schema = $to_schema)
MERGE (source)-[relationship:FK {
    _owner: $relationship_owner,
    from_column: $from_column,
    to_column: $to_column
}]->(target)
SET relationship.description = $description
RETURN source.name AS from_table, target.name AS to_table
