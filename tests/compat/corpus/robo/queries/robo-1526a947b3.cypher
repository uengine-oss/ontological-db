MATCH (a:JourneyStep {id: $a}), (b:JourneyStep {id: $b}) MERGE (a)-[r:NEXT]->(b) ON CREATE SET r.condition = $cond, r.source = 'proposal', r.createdAt = datetime()
