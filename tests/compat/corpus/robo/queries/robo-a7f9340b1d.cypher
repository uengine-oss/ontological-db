MATCH (am:ActivityMapping {session_id: $sid, task_id: $tid, rule_id: $rid}) SET am.reviewed = true RETURN am.score AS score, am.method AS method
