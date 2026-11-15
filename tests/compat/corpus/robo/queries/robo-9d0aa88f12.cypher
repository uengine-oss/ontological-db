MATCH (bc:BoundedContext {id: $bcId})-[:HAS_AGGREGATE]->(agg:Aggregate) RETURN agg.id AS id LIMIT 1
