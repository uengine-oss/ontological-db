MATCH (r:Rule {session_id: $sid}) RETURN r ORDER BY r.source_function, r.id
