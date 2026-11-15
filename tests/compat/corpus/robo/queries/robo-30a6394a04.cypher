MATCH (cmd:Command {id: $cmd_id})<-[:HAS_COMMAND]-(agg:Aggregate)<-[:HAS_AGGREGATE]-(bc:BoundedContext)
RETURN {
    id: cmd.id, name: cmd.name, displayName: cmd.displayName,
    actor: cmd.actor, aggregateName: agg.name, bcName: bc.name
} as info
