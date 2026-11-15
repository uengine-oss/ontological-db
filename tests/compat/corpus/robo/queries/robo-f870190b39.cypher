MERGE (n:Policy {id: $target_id})
SET n.name = $name, n.description = $description, n.createdAt = datetime()
WITH n
MATCH (bc:BoundedContext {id: $bc_id})
MERGE (bc)-[:HAS_POLICY]->(n)
RETURN n.id as id
