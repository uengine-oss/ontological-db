MATCH (inv:Invariant {id: $iid}), (cmd:Command {id: $cid}) MERGE (inv)-[r:VERIFIED_BY]->(cmd) ON CREATE SET r.createdAt = datetime()
