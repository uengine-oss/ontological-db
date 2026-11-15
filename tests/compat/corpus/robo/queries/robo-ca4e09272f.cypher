MATCH (p:BpmProcess {session_id: $sid})
OPTIONAL MATCH (p)-[:HAS_TASK]->(t:BpmTask {session_id: $sid})
WITH p, collect(DISTINCT t.id) AS task_ids
OPTIONAL MATCH (p)-[:HAS_ACTOR]->(a:BpmActor {session_id: $sid})
RETURN p, task_ids, collect(DISTINCT a.id) AS actor_ids
ORDER BY p.name
