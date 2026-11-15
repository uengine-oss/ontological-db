MATCH (n:_Entity:Customer {name: $name}) RETURN n._source_id AS k LIMIT 1
