MATCH (:BoundedContext {proposalSource: $pid})-[:HAS_AGGREGATE]->(:Aggregate {proposalSource: $pid})
RETURN count(*) AS c
