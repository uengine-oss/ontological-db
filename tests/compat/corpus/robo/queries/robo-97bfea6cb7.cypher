MATCH (p:BpmProcess {id: $pid, session_id: $sid}), (g:BpmGateway {id: $gid, session_id: $sid}) MERGE (p)-[:HAS_GATEWAY]->(g)
