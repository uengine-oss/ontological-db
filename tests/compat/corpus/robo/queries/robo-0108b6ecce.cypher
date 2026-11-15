MATCH (:BpmTask {session_id: $sid})-[:REALIZED_BY]->(r:Rule {session_id: $sid})
RETURN DISTINCT r.id AS rid
