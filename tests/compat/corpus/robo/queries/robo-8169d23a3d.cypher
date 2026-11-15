MATCH (bc:BoundedContext {id: $bc_id})
MERGE (agg:Aggregate {key: $key})
ON CREATE SET agg.id = $id,
              agg.createdAt = datetime()
SET agg.key = $key,
    agg.name = $name,
    agg.rootEntity = $root_entity,
    agg.description = $description,
    agg.updatedAt = datetime()
WITH bc, agg
MERGE (bc)-[:HAS_AGGREGATE {isPrimary: false}]->(agg)
