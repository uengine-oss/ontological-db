MATCH (us:UserStory)-[:IMPLEMENTS]->(target)
WHERE (target:BoundedContext AND target.id = $id)
   OR (target:Aggregate AND EXISTS {
           MATCH (bc:BoundedContext {id: $id})-[:HAS_AGGREGATE]->(target)
       })
OPTIONAL MATCH (us)-[:IMPLEMENTS]->(agg:Aggregate)
RETURN DISTINCT us.id AS id, us.role AS role, us.action AS action,
       us.benefit AS benefit, us.priority AS priority,
       us.acceptanceCriteria AS criteria,
       head(collect(DISTINCT agg.id)) AS aggregate_id
ORDER BY us.id
