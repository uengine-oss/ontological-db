MATCH (p:Policy {session_id: $sid}) WHERE p.kind = 'cross_bc' RETURN count(p) AS c
