MATCH (a:BpmActor {session_id: $sid}) RETURN a ORDER BY a.name
