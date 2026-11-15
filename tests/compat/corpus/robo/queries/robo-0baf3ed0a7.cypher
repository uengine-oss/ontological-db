MATCH (bc:BoundedContext)-[:HAS_AGGREGATE]->(agg:Aggregate {id: $agg_id}) RETURN bc.id AS bcId
