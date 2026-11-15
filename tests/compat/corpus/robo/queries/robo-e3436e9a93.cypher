MERGE (tbl:ExternalTable {name: $name, session_id: $sid}) WITH tbl MATCH (r:Rule {id: $rid, session_id: $sid}) MERGE (r)-[rel:EVALUATES]->(tbl) SET rel.direction = $direction
