MATCH (source:TABLE {name: $from_table})
      -[relationship:FK]->
      (target:TABLE {name: $to_table})
WHERE source._owner = $owner AND target._owner = $owner
  AND relationship._owner = $relationship_owner
  AND ($from_schema = '' OR source.schema = $from_schema)
  AND ($to_schema = '' OR target.schema = $to_schema)
  AND relationship.from_column = $from_column
  AND relationship.to_column = $to_column
DELETE relationship
RETURN count(*) AS deleted
