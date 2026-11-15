MATCH (bc:BoundedContext {id: $bc_id})-[:HAS_AGGREGATE]->(agg:Aggregate)
OPTIONAL MATCH (agg)-[:HAS_COMMAND]->(cmd:Command)
WITH agg, collect(DISTINCT cmd {.id, .name}) as commands
RETURN {
    id: agg.id,
    name: agg.name,
    displayName: agg.displayName,
    rootEntity: agg.rootEntity,
    invariants: agg.invariants,
    enumerations: agg.enumerations,
    valueObjects: agg.valueObjects,
    commands: commands
} as aggregate
ORDER BY agg.name
