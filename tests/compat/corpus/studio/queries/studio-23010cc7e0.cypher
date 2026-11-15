MATCH (n:_Entity:Customer) WHERE n.email IS NULL RETURN elementId(n) AS eid, n._source_id AS key, n.name AS name, n._source_system AS source_system LIMIT 500
