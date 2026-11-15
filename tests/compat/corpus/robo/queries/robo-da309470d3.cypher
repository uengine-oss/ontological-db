MATCH (n:BpmTask) WHERE n.session_id IS NOT NULL WITH n, count(n) AS c DETACH DELETE n RETURN c
