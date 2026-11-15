MATCH (a)-[r:`PLACED`]->(b) RETURN labels(a) AS a, labels(b) AS b LIMIT 1
