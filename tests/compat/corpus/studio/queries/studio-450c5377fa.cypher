MATCH (n:_Entity) OPTIONAL MATCH (n)-[r]->(m:_Entity) RETURN n, r, m LIMIT $limit
