UNWIND $keywords as keyword
MATCH (bc:BoundedContext)
OPTIONAL MATCH (bc)-[:HAS_AGGREGATE]->(agg:Aggregate)
WITH bc, agg, keyword,
     CASE
         WHEN toLower(bc.name) CONTAINS toLower(keyword) THEN 3
         WHEN toLower(coalesce(bc.description, '')) CONTAINS toLower(keyword) THEN 2
         WHEN agg IS NOT NULL AND toLower(agg.name) CONTAINS toLower(keyword) THEN 1
         ELSE 0
     END as score
WHERE score > 0
WITH bc, sum(score) as totalScore
ORDER BY totalScore DESC
LIMIT 1
RETURN bc.id as id, bc.name as name, totalScore as score
