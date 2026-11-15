MATCH (n {id: $node_id})
RETURN labels(n)[0] as nodeType, n as node
