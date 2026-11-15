MATCH (agg:Aggregate)-[:HAS_INVARIANT]->(inv:Invariant {id: $iid})
MATCH (agg)-[:HAS_COMMAND]->(cmd:Command {id: $cid})
MERGE (inv)-[r:VERIFIED_BY]->(cmd)
  ON CREATE SET r.createdAt = datetime()
RETURN inv.id AS id
