MATCH (n {id: $node_id})
RETURN n {.*, labels: labels(n)} as node
