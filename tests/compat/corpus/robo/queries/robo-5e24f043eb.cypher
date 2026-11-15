MERGE (am:ActivityMapping {id: $id, session_id: $sid}) SET am.task_id = $tid, am.rule_id = $rid, am.score = $score,     am.method = $method, am.reviewed = false
