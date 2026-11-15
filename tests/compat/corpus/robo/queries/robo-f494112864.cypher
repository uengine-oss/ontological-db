MATCH (inv:Invariant)
WHERE NOT (:Aggregate)-[:HAS_INVARIANT]->(inv)
DETACH DELETE inv
RETURN count(inv) AS pruned
