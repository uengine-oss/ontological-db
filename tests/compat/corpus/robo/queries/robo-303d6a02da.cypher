MATCH (p:BpmProcess {id: $pid, session_id: $sid}), (a:BpmActor {id: $aid, session_id: $sid}) MERGE (p)-[:HAS_ACTOR]->(a)
