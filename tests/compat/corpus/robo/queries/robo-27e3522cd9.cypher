MATCH (bc:BoundedContext {id: $bid}), (jn:Journey {id: $jid}) MERGE (bc)-[r:HAS_JOURNEY]->(jn) ON CREATE SET r.createdAt = datetime()
