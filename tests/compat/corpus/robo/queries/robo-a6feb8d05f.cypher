MATCH (op {_id: $fid, _owner: 'analyzer'})-[:PARENT_OF*0..]->(_n)-[r:READS|WRITES]->(t:TABLE)
OPTIONAL MATCH (t)-[:HAS_COLUMN]->(c:COLUMN)
WITH t, type(r) AS access,
     collect(DISTINCT {name: c.name, dtype: c.data_type, pk: c.primary_key}) AS columns
RETURN access, t.name AS table_name, columns
ORDER BY t.name
