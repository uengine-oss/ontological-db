MATCH (b:BoundedContext {session_id: $sid}) RETURN b.id AS id, b.key AS key ORDER BY b.key LIMIT 1
