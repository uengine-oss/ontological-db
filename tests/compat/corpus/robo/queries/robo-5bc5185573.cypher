MATCH (a:BpmTask {id: $a, session_id: $sid}), (b:BpmTask {id: $b, session_id: $sid}) MERGE (a)-[:NEXT]->(b)
