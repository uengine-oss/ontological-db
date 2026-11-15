MATCH (bc:BoundedContext {id: $bid}), (pol:Policy {id: $pid}) MERGE (bc)-[r:HAS_POLICY]->(pol) ON CREATE SET r.createdAt = datetime()
