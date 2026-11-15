MATCH (a:BpmActor {id: $aid, session_id: $sid}), (t:BpmTask {id: $tid, session_id: $sid}) MERGE (a)-[:PERFORMS]->(t)
