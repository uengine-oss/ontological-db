MATCH (agg:Aggregate {id: $aggregate_id})
OPTIONAL MATCH (agg)-[:HAS_PROPERTY]->(prop:Property)
WITH agg, collect(DISTINCT prop {
    .id, .name, .type, .description, .isKey, .isForeignKey, .isRequired
}) as properties
RETURN {
    id: agg.id,
    name: agg.name,
    rootEntity: agg.rootEntity,
    invariants: agg.invariants,
    properties: properties
} as aggregate
