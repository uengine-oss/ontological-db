MATCH (t:BpmTask {session_id: $sid})
OPTIONAL MATCH (a:BpmActor {session_id: $sid})-[:PERFORMS]->(t)
WITH t, collect(DISTINCT a.id) AS actor_ids
RETURN t, actor_ids
ORDER BY t.sequence_index
