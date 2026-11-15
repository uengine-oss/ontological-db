MATCH (a:_Entity {_source_id: $a}) MATCH (b:_Entity {_source_id: $b}) RETURN count(a) AS na, count(b) AS nb
