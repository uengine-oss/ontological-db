MATCH (e:Event)
WHERE NOT (()-[:TRIGGERED_BY]->(e))
  AND NOT e.name ENDS WITH 'Failed'
OPTIONAL MATCH (bc:BoundedContext)-[:HAS_EVENT]->(e)
RETURN e.name AS event, bc.name AS bc
ORDER BY bc, e.name
