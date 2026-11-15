MATCH (r:Rule {id: $rid, session_id: $sid}) SET r.es_role = $role, r.es_role_confidence = 1.0 RETURN r.id AS id
