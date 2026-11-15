MATCH (evt:Event {id: $source_id})
MATCH (pol:Policy {id: $target_id})
MERGE (evt)-[:TRIGGERS]->(pol)
RETURN evt.id as id
