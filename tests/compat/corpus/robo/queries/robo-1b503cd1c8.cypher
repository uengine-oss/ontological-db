MATCH (bc:BoundedContext {id: $bc_id})
MERGE (pol:Policy {id: $pol_id})
SET pol.name = $name,
    pol.description = $description,
    pol.createdAt = datetime()
MERGE (bc)-[:HAS_POLICY]->(pol)
