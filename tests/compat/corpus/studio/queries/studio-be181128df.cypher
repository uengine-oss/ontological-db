MATCH (t1:Table)-[:HAS_COLUMN]->(c1:Column)-[:REFERENCES]->(c2:Column)<-[:HAS_COLUMN]-(t2:Table)
WHERE (t1.datasource = $datasource OR t1.db = $datasource)
  AND t1.schema = $schema
RETURN t1.name AS from_table, c1.name AS from_column,
       t2.name AS to_table, c2.name AS to_column
