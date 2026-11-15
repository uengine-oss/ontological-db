MATCH (evt:Event {id: $source_id})
MATCH (pol:Policy {id: $target_id})
MERGE (evt)-[:TRIGGERS {priority: 1, isEnabled: true}]->(pol)
RETURN evt.id as id
