UNWIND $keywords as keyword
MATCH (n)
WHERE (
    ($nodeTypes IS NULL OR any(t IN $nodeTypes WHERE t IN labels(n)))
)
AND (n:Command OR n:Event OR n:Policy OR n:Aggregate)
AND (
    toLower(n.name) CONTAINS toLower(keyword)
    OR toLower(coalesce(n.description, '')) CONTAINS toLower(keyword)
)
AND NOT n.id IN $excludeIds

// Get the BC for each node
OPTIONAL MATCH (bc:BoundedContext)-[:HAS_AGGREGATE|HAS_POLICY*1..3]->(n)

WITH DISTINCT n, bc,
     CASE
         WHEN toLower(n.name) CONTAINS toLower($primary_keyword) THEN 1.0
         WHEN toLower(n.name) CONTAINS toLower($query) THEN 0.9
         ELSE 0.7
     END as score

RETURN {
    id: n.id,
    name: n.name,
    type: labels(n)[0],
    bcId: bc.id,
    bcName: bc.name,
    description: n.description,
    similarity: score
} as result
ORDER BY score DESC
LIMIT $limit
