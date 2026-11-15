MATCH (a {session_id: $sid})-[r:NEXT]->(b {session_id: $sid}) RETURN count(r) AS c
