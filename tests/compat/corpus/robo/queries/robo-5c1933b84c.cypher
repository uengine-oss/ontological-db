MATCH (a:BpmActor {id: $aid, session_id: $sid}), (g:BpmGateway {id: $gid, session_id: $sid}) MERGE (a)-[:PERFORMS]->(g)
