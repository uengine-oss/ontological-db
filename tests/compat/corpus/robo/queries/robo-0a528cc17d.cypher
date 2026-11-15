MATCH (q:BpmSequence {id: $qid, session_id: $sid}), (t:BpmTask {id: $tid, session_id: $sid}) MERGE (q)-[:CONTAINS]->(t)
