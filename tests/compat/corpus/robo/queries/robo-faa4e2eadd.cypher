MATCH (n:BpmTask {session_id: $sid}) WITH n, count(n) AS c DETACH DELETE n RETURN c
