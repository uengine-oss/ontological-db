MATCH (evt:Event {id: $event_id})-[:TRIGGERS]->(pol:Policy)<-[:HAS_POLICY]-(bc:BoundedContext)
RETURN DISTINCT bc.id as bcId
