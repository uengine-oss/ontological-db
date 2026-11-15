MATCH (t1:Table)-[r:FK_TO_TABLE]->(t2:Table)
WHERE (t1.datasource = $datasource OR t1.db = $datasource)
  AND t1.schema = $schema
RETURN t1.name AS from_table,
       coalesce(r.sourceColumn, r.from_column, '') AS from_column,
       t2.name AS to_table,
       coalesce(r.targetColumn, r.to_column, '') AS to_column
