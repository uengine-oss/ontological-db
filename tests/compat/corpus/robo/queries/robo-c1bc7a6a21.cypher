MERGE (am:ActivityMapping {id: $id, session_id: $sid}) SET am.task_id = $tid, am.rule_id = $rid, am.score = $conf,     am.method = 'manual', am.reviewed = true, am.evidence_role = 'primary'
