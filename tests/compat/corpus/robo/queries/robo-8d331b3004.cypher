MATCH (t:BpmTask {session_id: $sid})-[link:SOURCED_FROM]->(p:DocumentPassage {session_id: $sid})
RETURN t.id AS task_id, p, link.score AS score, link.rank AS rank, link.low_confidence AS low_confidence
ORDER BY t.id, link.rank
