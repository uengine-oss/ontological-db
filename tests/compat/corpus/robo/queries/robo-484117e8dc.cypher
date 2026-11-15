MATCH (cmd:Command)<-[:HAS_COMMAND]-(agg:Aggregate)<-[:HAS_AGGREGATE]-(bc:BoundedContext)
WHERE NOT (:Policy)-[:INVOKES]->(cmd)
RETURN {
    id: cmd.id,
    name: cmd.name,
    displayName: cmd.displayName,
    actor: cmd.actor,
    aggregateId: agg.id,
    aggregateName: agg.name,
    bcId: bc.id,
    bcName: bc.name
} as startCommand
ORDER BY bc.name, cmd.name
