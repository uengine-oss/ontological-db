MATCH (r:Rule {id: $rid, session_id: $sid}) SET r.es_role = $role, r.es_role_confidence = $conf RETURN r.id AS id
