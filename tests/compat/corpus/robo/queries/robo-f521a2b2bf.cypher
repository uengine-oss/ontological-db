MATCH (pol:Policy {id: $pid}), (cmd:Command {id: $cid}) MERGE (pol)-[r:INVOKES]->(cmd) ON CREATE SET r.isAsync = true, r.createdAt = datetime()
