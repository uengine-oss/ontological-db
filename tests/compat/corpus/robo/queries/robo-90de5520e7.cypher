MATCH (n:BpmTask {session_id: $sid}) RETURN count(n) AS c
