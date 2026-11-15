MATCH (p:BpmProcess {id: $pid, session_id: $sid}), (t:BpmTask {id: $tid, session_id: $sid}) MERGE (p)-[:HAS_TASK]->(t)
