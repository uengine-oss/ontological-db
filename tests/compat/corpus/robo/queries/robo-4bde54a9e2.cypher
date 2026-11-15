MATCH (t:BpmTask {session_id: $sid})-[link:REALIZED_BY]->(rule:Rule {session_id: $sid}) RETURN t.id AS tid, rule.id AS rid, link.confidence AS conf
