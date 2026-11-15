MATCH (n:_Entity:Customer) WHERE n.grade IS NOT NULL RETURN elementId(n) AS eid, n._source_id AS key, n.name AS name, n._source_system AS source_system, n.grade AS value LIMIT 2000
