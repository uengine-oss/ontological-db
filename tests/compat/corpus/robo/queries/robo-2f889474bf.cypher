MATCH (bcU:BoundedContext)-[:HAS_AGGREGATE]->(:Aggregate)-[:HAS_COMMAND]->(:Command)-[:EMITS]->(e:Event)
MATCH (e)-[:TRIGGERS]->(p:Policy)<-[:HAS_POLICY]-(bcD:BoundedContext)
WHERE bcU.id <> bcD.id AND (bcU.id = $id OR bcD.id = $id)
RETURN bcU.id AS from_id, bcU.name AS from_name,
       bcD.id AS to_id, bcD.name AS to_name,
       e.name AS message
