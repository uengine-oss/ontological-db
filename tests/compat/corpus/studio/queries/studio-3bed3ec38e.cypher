MATCH (a:_Entity)-[:SAME_AS]->(b:_Entity) RETURN a._source_id AS a, b._source_id AS b
