MATCH (a:Aggregate {id: $id})-[:HAS_COMMAND]->(:Command)-[:EMITS]->(e:Event)
OPTIONAL MATCH (e)-[:TRIGGERS]->(p:Policy)
WITH DISTINCT p WHERE p IS NOT NULL
RETURN p.id AS id, p.name AS name, p.description AS description,
       p.condition AS condition
ORDER BY p.name
