MATCH (t:BpmTask {id: $tid, session_id: $sid}), (d:DocumentPassage {id: $pid, session_id: $sid}) MERGE (t)-[rel:SOURCED_FROM]->(d) SET rel.score = $score, rel.rank = $rank, rel.low_confidence = $lowc
