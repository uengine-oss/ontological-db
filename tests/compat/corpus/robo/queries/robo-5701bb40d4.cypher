MATCH (bc:BoundedContext {id: $id})-[:HAS_AGGREGATE]->(a:Aggregate)
RETURN a.id AS id, a.name AS name, a.description AS description,
       a.rootEntity AS rootEntity, a.invariants AS invariants
ORDER BY a.name
