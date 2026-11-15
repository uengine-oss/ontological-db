MATCH (x {graph_owner:$graph_owner})-[:CALLS]-(y {graph_owner:$graph_owner}) WHERE x.id IN $ids AND y.id IS NOT NULL RETURN DISTINCT y.id AS id
