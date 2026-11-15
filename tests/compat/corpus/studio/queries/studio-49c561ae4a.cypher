MATCH (a:_Entity {_source_id: $a})-[r:`PLACED`]->(b:_Entity {_source_id: $b}) RETURN count(r) AS c
