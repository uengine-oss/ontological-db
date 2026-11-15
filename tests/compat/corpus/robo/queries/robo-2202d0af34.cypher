MATCH (bc:BoundedContext)
OPTIONAL MATCH (bc)-[:HAS_AGGREGATE]->(agg:Aggregate)
OPTIONAL MATCH (us:UserStory)-[:IMPLEMENTS]->(bc)
WITH bc, count(DISTINCT agg) as aggregateCount, count(DISTINCT us) as userStoryCount
RETURN {
    id: bc.id,
    name: bc.name,
    displayName: bc.displayName,
    description: bc.description,
    owner: bc.owner,
    domainType: bc.domainType,
    userStoryIds: bc.userStoryIds,
    aggregateCount: aggregateCount,
    userStoryCount: userStoryCount
} as context
ORDER BY bc.name
