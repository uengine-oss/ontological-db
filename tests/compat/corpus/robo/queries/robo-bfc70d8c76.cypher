MATCH (bc:BoundedContext {id: $bid}), (rm:ReadModel {id: $rid}) MERGE (bc)-[r:HAS_READMODEL]->(rm) ON CREATE SET r.createdAt = datetime()
