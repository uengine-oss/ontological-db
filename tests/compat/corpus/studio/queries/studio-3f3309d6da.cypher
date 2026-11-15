MATCH (n:_Entity:`Customer`) RETURN n._source_id AS key, n.name AS name, n._source_system AS source_system, n.`email` AS `p_email` LIMIT 5000
