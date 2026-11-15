MATCH (n:_Entity:`Customer`) WHERE n.`grade` IS NULL RETURN n._source_id AS key, n.name AS name LIMIT 500
