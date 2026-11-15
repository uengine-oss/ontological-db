MATCH (us:UserStory {id: $us}), (f:Feature {id: $fid}) MERGE (f)-[r:HAS_USER_STORY]->(us) ON CREATE SET r.createdAt = datetime(), r.source = 'proposal'
