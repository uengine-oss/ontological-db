MERGE (agg:Aggregate {id: $agg_id})
SET agg.name = $name,
    agg.rootEntity = $name,
    agg.description = $description,
    agg.createdAt = datetime()
WITH agg
OPTIONAL MATCH (bc:BoundedContext {id: $bc_id})
WHERE bc IS NOT NULL
MERGE (bc)-[:HAS_AGGREGATE]->(agg)
WITH agg
MATCH (us:UserStory {id: $us_id})
MERGE (us)-[:IMPLEMENTS]->(agg)
RETURN agg.id as id
