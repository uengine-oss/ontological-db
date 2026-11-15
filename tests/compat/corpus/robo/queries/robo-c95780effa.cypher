MATCH (a)-[r]->(b) WHERE a.session_id = $sid AND b.session_id = $sid RETURN a.id AS src, b.id AS dst, type(r) AS rel
