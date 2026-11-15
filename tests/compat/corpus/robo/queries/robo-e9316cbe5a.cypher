MATCH (bc:BoundedContext) WHERE bc.id IN $seeds
OPTIONAL MATCH (bc)-[:HAS_AGGREGATE]->(:Aggregate)-[:HAS_COMMAND]->(:Command)-[:EMITS]->(evt:Event)
              -[:TRIGGERS]->(:Policy)<-[:HAS_POLICY]-(otherBc:BoundedContext)
RETURN collect(DISTINCT bc.id) + collect(DISTINCT otherBc.id) AS allIds
