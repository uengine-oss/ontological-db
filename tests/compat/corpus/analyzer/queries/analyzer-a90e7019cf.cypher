MATCH (a {graph_owner:$graph_owner})-[:CALLS]->(b {id:$id, graph_owner:$graph_owner}) WHERE a.id IS NOT NULL RETURN a.id AS id, coalesce(a.name,'') AS name, 'CALLS' AS relation, 'in' AS direction
