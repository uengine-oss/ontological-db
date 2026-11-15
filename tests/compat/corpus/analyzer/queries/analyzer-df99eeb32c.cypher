MATCH (a {graph_owner: 'analyzer'})-[:CALLS]->(n {graph_owner: 'analyzer'}) WHERE n.id IS NOT NULL RETURN n.id AS id, count(DISTINCT a) AS fi
