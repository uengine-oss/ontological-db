MATCH (r:Rule {id: $rid, session_id: $sid}) SET r.context_cluster = $ctx RETURN r.id AS id
