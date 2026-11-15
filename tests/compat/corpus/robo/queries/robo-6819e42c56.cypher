MATCH (a:Aggregate {id: $id})-[:HAS_COMMAND]->(c:Command)
OPTIONAL MATCH (c)-[:EMITS]->(e:Event)
WITH c, collect(DISTINCT e.name) AS events
RETURN c.id AS id, c.name AS name, c.description AS description,
       c.preconditions AS preconditions, c.postconditions AS postconditions,
       events AS events_emitted
ORDER BY c.name
