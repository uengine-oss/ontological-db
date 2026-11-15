MATCH (agg:Aggregate {id: $aid}), (cmd:Command {id: $cid}) MERGE (agg)-[r:HAS_COMMAND]->(cmd) ON CREATE SET r.createdAt = datetime()
