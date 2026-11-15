MATCH (bc:BoundedContext)-[:HAS_AGGREGATE]->(a:Aggregate {id: $id})
RETURN bc.id AS bc_id
