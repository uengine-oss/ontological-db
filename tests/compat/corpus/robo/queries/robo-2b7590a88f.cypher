MATCH (n:BpmTask {session_id: $sid}) WHERE size(labels(n)) = 1 WITH n, count(n) AS c DETACH DELETE n RETURN c
