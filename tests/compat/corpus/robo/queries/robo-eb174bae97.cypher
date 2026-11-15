MATCH (n {id: $id})
OPTIONAL MATCH (bc:BoundedContext)-[:HAS_AGGREGATE|HAS_POLICY|HAS_READMODEL|HAS_UI*1..3]->(n)
WITH n, bc
RETURN CASE WHEN n:BoundedContext THEN n.id ELSE coalesce(bc.id, n.bcId) END AS bcId
LIMIT 1
