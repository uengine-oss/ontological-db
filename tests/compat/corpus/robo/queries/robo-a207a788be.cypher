MATCH (a:Aggregate)
WHERE a.id = $id
   OR (a)-[:HAS_COMMAND]->({id: $id})
   OR (a)-[:HAS_COMMAND]->(:Command)-[:EMITS]->({id: $id})
RETURN a.id AS aggId
LIMIT 1
