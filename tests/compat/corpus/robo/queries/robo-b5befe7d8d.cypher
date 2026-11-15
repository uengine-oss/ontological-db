MATCH (t:BpmTask {session_id: $sid})
OPTIONAL MATCH (t)-[:REALIZED_BY]->(r:Rule {session_id: $sid})
WITH t, count(r) AS rc
RETURN count(*) AS total_tasks,
       count(CASE WHEN rc > 0 THEN 1 END) AS mapped_tasks,
       count(CASE WHEN rc = 0 THEN 1 END) AS zero_rule_tasks
