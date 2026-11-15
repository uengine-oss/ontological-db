MATCH (bc:BoundedContext {id: $bc_id})-[:HAS_POLICY]->(pol:Policy)
RETURN pol.id as id, pol.name as name
ORDER BY pol.name
