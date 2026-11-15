MATCH (bc:BoundedContext)
OPTIONAL MATCH (bc)-[:HAS_AGGREGATE]->(agg:Aggregate)
OPTIONAL MATCH (agg)-[:HAS_PROPERTY]->(prop:Property)
WITH bc, agg, collect(DISTINCT prop {
    .id, .name, .displayName, .type, .description, .isKey, .isForeignKey, .isRequired
}) as properties
ORDER BY bc.name, agg.name
RETURN {
    bcId: bc.id,
    bcName: bc.name,
    bcDisplayName: bc.displayName,
    bcDescription: bc.description,
    aggregateId: agg.id,
    aggregateName: agg.name,
    aggregateDisplayName: agg.displayName,
    rootEntity: agg.rootEntity,
    invariants: agg.invariants,
    enumerations: agg.enumerations,
    valueObjects: agg.valueObjects,
    properties: properties
} as item
