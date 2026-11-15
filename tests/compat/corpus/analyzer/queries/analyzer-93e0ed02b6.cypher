MATCH (x {id:$id, graph_owner:$graph_owner})-[:CALLS]->(b {graph_owner:$graph_owner}) WHERE b.id IS NOT NULL RETURN b.id AS id, coalesce(b.name,'') AS name
