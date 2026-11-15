MATCH (a:Aggregate {id: $id})-[:HAS_COMMAND]->(:Command)-[:EMITS]->(e:Event)
RETURN DISTINCT e.id AS id, e.name AS name, e.description AS description
ORDER BY e.name
