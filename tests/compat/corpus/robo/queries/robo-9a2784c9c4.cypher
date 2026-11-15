OPTIONAL MATCH (inv:Invariant {id: $iid})
OPTIONAL MATCH (cmd:Command {id: $cid})
OPTIONAL MATCH (agg:Aggregate)-[:HAS_INVARIANT]->(inv)
RETURN inv IS NOT NULL AS invExists,
       cmd IS NOT NULL AS cmdExists,
       (inv IS NOT NULL AND cmd IS NOT NULL
          AND EXISTS { (agg)-[:HAS_COMMAND]->(cmd) }) AS sameAgg,
       (inv IS NOT NULL AND cmd IS NOT NULL
          AND EXISTS { (inv)-[:VERIFIED_BY]->(cmd) }) AS already
