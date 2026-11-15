MERGE (pol:Policy {id: $pol_id})
SET pol.name = $name,
    pol.description = $description,
    pol.createdAt = datetime()
WITH pol
OPTIONAL MATCH (bc:BoundedContext {id: $bc_id})
WHERE bc IS NOT NULL
MERGE (bc)-[:HAS_POLICY]->(pol)
RETURN pol.id as id
