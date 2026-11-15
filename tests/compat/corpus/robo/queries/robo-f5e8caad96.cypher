MATCH (n {id: $id})
OPTIONAL MATCH (n)<-[:HAS_READMODEL|HAS_POLICY|HAS_COMMAND|HAS_AGGREGATE]-(bc:BoundedContext)
OPTIONAL MATCH (bc2:BoundedContext {id: $id})
OPTIONAL MATCH (us_bc:UserStory)-[:IMPLEMENTS]->(n) WITH n, coalesce(bc, bc2) AS found_bc, us_bc
OPTIONAL MATCH (us_bc)-[:IMPLEMENTS]->(bc3:BoundedContext)
WITH n, coalesce(found_bc, bc3) AS bc WHERE bc IS NOT NULL
RETURN DISTINCT bc.id AS id, bc.name AS name
