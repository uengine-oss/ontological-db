MATCH (w:CQRSWhere {id: $where_id})
DETACH DELETE w
RETURN count(w) as deleted
