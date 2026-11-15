MATCH (cmd:Command {id: $cmd_id})-[:EMITS]->(evt:Event)
OPTIONAL MATCH (evt)-[:TRIGGERS]->(pol:Policy)-[:INVOKES]->(nextCmd:Command)
OPTIONAL MATCH (cmd)<-[:HAS_COMMAND]-(agg:Aggregate)<-[:HAS_AGGREGATE]-(bc:BoundedContext)
RETURN {
    cmdId: cmd.id, cmdName: cmd.name, cmdDisplayName: cmd.displayName,
    cmdActor: cmd.actor, cmdDescription: cmd.description,
    aggName: agg.name, bcName: bc.name,
    evtId: evt.id, evtName: evt.name, evtDisplayName: evt.displayName,
    evtDescription: evt.description,
    polId: pol.id, polName: pol.name, polDisplayName: pol.displayName,
    nextCmdId: nextCmd.id
} as step
