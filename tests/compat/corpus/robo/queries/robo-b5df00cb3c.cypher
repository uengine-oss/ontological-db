MERGE (a:BpmActor {id: $id, session_id: $sid}) SET a.name = $name, a.description = $description, a.process_id = $pid
