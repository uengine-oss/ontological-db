CALL db.schema.visualization() YIELD relationships RETURN [r IN relationships |   [head(labels(startNode(r))), type(r), head(labels(endNode(r)))]] AS edges
