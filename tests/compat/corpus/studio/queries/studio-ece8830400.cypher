MATCH (a)-[r]->(b) RETURN DISTINCT labels(a) AS a, type(r) AS t, labels(b) AS b LIMIT 200
