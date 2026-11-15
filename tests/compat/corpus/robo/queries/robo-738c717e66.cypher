MATCH (am:ActivityMapping {session_id: $sid, task_id: $tid, rule_id: $rid}) DETACH DELETE am
