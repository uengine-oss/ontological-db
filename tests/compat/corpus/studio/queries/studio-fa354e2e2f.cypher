MATCH (a:_Entity {_source_id: $a}) MATCH (b:_Entity {_source_id: $b}) MATCH p = (a)-[*2..3]-(b) RETURN [r IN relationships(p) | coalesce(r._confidence, 0.7)] AS confs LIMIT 50
