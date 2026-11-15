MATCH (evt:Event {id: $eid}), (pol:Policy {id: $pid}) MERGE (evt)-[r:TRIGGERS]->(pol) ON CREATE SET r.priority = 1, r.isEnabled = true, r.createdAt = datetime()
