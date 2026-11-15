MATCH (cmd:Command {id: $cmd_id})-[:EMITS]->(evt:Event)
OPTIONAL MATCH (evt)-[:TRIGGERS]->(pol:Policy)-[:INVOKES]->(nextCmd:Command)
RETURN {
    commandId: cmd.id,
    commandName: cmd.name,
    commandDisplayName: cmd.displayName,
    commandActor: cmd.actor,
    eventId: evt.id,
    eventName: evt.name,
    eventDisplayName: evt.displayName,
    nextCommandId: nextCmd.id
} as step
