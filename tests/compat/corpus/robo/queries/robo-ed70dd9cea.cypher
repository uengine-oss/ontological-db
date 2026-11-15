MATCH (am:ActivityMapping {session_id: $sid, task_id: $tid}) DETACH DELETE am
