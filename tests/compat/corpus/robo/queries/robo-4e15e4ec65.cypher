MATCH (t:BpmTask {id: $tid, session_id: $sid}), (r:Rule {id: $rid, session_id: $sid}) RETURN count(t) AS tc, count(r) AS rc
