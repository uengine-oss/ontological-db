MATCH (n:_Entity) WHERE elementId(n) = $node_id OPTIONAL MATCH (n)-[r]-(m:_Entity) RETURN n, r, m
