MATCH (agg:Aggregate {id: $aid})-[:HAS_INVARIANT]->(inv:Invariant)
RETURN coalesce(max(inv.seq), 0) AS maxSeq
