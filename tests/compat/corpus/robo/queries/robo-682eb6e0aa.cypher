MATCH (t:BpmTask {session_id: $sid}) OPTIONAL MATCH (a:BpmActor {session_id: $sid})-[:PERFORMS]->(t) RETURN t, collect(DISTINCT a.name) AS actor_names ORDER BY t.sequence_index
