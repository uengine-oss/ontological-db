MATCH (q:BpmSequence {session_id: $sid})
OPTIONAL MATCH (q)-[:CONTAINS]->(t:BpmTask)
WITH q, collect(t.id) AS task_ids
RETURN q, task_ids
