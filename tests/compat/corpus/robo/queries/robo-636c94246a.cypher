MATCH (bc:BoundedContext)
OPTIONAL MATCH (bc)-[:HAS_AGGREGATE]->(agg:Aggregate)
WITH bc, collect(DISTINCT agg {.id, .name}) as aggregates
RETURN {
    id: bc.id,
    name: bc.name,
    displayName: bc.displayName,
    description: bc.description,
    owner: bc.owner,
    domainType: bc.domainType,
    userStoryIds: bc.userStoryIds,
    aggregates: aggregates
} as bounded_context
ORDER BY bc.name
