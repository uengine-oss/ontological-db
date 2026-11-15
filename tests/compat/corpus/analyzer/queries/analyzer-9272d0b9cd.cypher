MATCH (n {id:$id, graph_owner:$graph_owner}) RETURN labels(n) AS labels, properties(n) AS props
