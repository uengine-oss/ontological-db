MATCH (agg:Aggregate)-[:HAS_COMMAND]->(:Command)-[:EMITS]->(:Event {id: $e}) RETURN agg.id AS id LIMIT 1
