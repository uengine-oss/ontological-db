MATCH (n:_Entity) WHERE $class IN labels(n) OPTIONAL MATCH (n)-[r]->(m:_Entity) RETURN n, r, m LIMIT $limit
