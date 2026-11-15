MATCH (n:_Entity) WHERE any(lbl IN labels(n) WHERE lbl IN $classes) OPTIONAL MATCH (n)-[r]->(m:_Entity) RETURN n, r, m LIMIT $limit
