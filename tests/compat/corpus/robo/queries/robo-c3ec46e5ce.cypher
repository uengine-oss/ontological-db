MATCH (t:BpmTask {id: $tid, session_id: $sid}) SET t.conditions = $conds
