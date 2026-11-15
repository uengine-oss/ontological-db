MATCH (us:UserStory {id: $us}), (bc:BoundedContext {id: $bid}) MERGE (us)-[:IMPLEMENTS]->(bc)
