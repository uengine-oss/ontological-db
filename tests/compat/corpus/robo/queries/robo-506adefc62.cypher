MATCH (bc:BoundedContext {id: $bc_id})
MATCH (n:Aggregate {id: $id})
MERGE (bc)-[:HAS_AGGREGATE]->(n)
