UNWIND $rows AS r
MATCH (bc:BoundedContext {id: r.bc_id})
MERGE (agg:Aggregate {key: r.key})
  ON CREATE SET agg.id = randomUUID(),
                agg.createdAt = datetime()
SET agg.key = r.key,
    agg.name = r.name,
    agg.displayName = r.display_name,
    agg.rootEntity = r.root_entity,
    agg.invariants = r.invariants,
    agg.enumerations = r.enumerations_json,
    agg.valueObjects = r.value_objects_json,
    agg.updatedAt = datetime()
MERGE (bc)-[:HAS_AGGREGATE {isPrimary: false}]->(agg)
RETURN {
  id: agg.id,
  key: agg.key,
  name: agg.name,
  displayName: agg.displayName,
  rootEntity: agg.rootEntity,
  invariants: agg.invariants,
  enumerations: agg.enumerations,
  valueObjects: agg.valueObjects
} AS result
