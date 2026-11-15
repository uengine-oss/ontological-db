MATCH (agg:Aggregate {id: $aggregate_id})
SET agg.enumerations = $enumerations_json,
    agg.valueObjects = $value_objects_json,
    agg.updatedAt = datetime()
RETURN {
    id: agg.id,
    name: agg.name,
    rootEntity: agg.rootEntity,
    invariants: agg.invariants,
    enumerations: agg.enumerations,
    valueObjects: agg.valueObjects
} as aggregate
LIMIT 1
