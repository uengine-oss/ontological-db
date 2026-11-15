MATCH (bc:BoundedContext {id: $bc_id})
MERGE (agg:Aggregate {key: $key})
ON CREATE SET agg.id = randomUUID(),
              agg.createdAt = datetime()
SET agg.key = $key,
    agg.name = $name,
    agg.displayName = $display_name,
    agg.rootEntity = $root_entity,
    agg.invariants = $invariants,
    agg.enumerations = $enumerations_json,
    agg.valueObjects = $value_objects_json,
    agg.updatedAt = datetime()
WITH bc, agg
MERGE (bc)-[:HAS_AGGREGATE {isPrimary: false}]->(agg)
WITH agg
RETURN {
    id: agg.id,
    key: agg.key,
    name: agg.name,
    displayName: agg.displayName,
    rootEntity: agg.rootEntity,
    invariants: agg.invariants,
    enumerations: agg.enumerations,
    valueObjects: agg.valueObjects
} as aggregate
LIMIT 1
