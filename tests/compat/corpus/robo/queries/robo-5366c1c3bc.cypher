MATCH (f:Feature)
WHERE NOT (:BoundedContext)-[:HAS_FEATURE]->(f)
DETACH DELETE f
RETURN count(f) AS pruned
