MATCH (t:BpmTask {id: $tid, session_id: $sid})-[rel:REALIZED_BY]->(r:Rule {id: $rid, session_id: $sid}) DELETE rel
