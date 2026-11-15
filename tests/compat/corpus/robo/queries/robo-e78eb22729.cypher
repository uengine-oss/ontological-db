MERGE (q:BpmSequence {id: $id, session_id: $sid}) SET q.name = $name, q.process_id = $pid
