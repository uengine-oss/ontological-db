MATCH (n) WHERE n.session_id = $sid AND (n:BpmTask OR n:BpmActor OR n:BpmSequence) RETURN n, labels(n) AS labels
