MATCH (bc:BoundedContext {id: $bid}), (agg:Aggregate {id: $aid}) MERGE (bc)-[r:HAS_AGGREGATE]->(agg) ON CREATE SET r.createdAt = datetime()
