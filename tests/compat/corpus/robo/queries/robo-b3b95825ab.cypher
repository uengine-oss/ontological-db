MATCH (n {id: $node_id}) RETURN n, labels(n)[0] AS nodeType LIMIT 1
