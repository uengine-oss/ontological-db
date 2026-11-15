MATCH (bc:BoundedContext {id:$id})-[:HAS_AGGREGATE]->(:Aggregate)
      -[:HAS_COMMAND]->(:Command)-[:EMITS]->(e:Event)
OPTIONAL MATCH (e)<-[:TRIGGERED_BY]-(p:Policy)<-[:HAS_POLICY]-(obc:BoundedContext)
WHERE obc.id <> $id
RETURN coalesce(obc.displayName, obc.name) AS other,
       coalesce(e.displayName, e.name) AS msg
LIMIT 50
