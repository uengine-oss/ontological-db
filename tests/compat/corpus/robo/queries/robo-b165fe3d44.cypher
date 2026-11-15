MATCH (us:UserStory {id: $us}), (d:Aggregate {id: $did}) MERGE (us)-[r:IMPLEMENTS]->(d) ON CREATE SET r.createdAt = datetime(), r.source = 'proposal'
