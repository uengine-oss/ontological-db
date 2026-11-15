MATCH (s:_Entity {_source_id: $key})-[r:PLACED]->() RETURN count(r) AS c
