MATCH (rule:Rule {session_id: $sid})-[rel:EVALUATES]->(tbl:ExternalTable {session_id: $sid}) RETURN rule.id AS rid, tbl.name AS tname, rel.direction AS dir
