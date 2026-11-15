MATCH (n:BpmTask) WHERE n.session_id IS NULL AND size(labels(n)) = 1 SET n.session_id = $sid RETURN count(n) AS c
