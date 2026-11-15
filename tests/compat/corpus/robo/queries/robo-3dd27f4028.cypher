MATCH (am:ActivityMapping {session_id: $sid, task_id: $tid, rule_id: $rid}) SET am.evidence_role = $role
