MATCH (t:BpmTask {id: $tid, session_id: $sid})-[rel:REALIZED_BY]->() DELETE rel
