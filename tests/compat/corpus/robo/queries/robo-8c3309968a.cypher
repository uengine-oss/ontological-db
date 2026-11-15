MATCH (evt:Event {name: $event_name})-[:TRIGGERS]->(pol:Policy)<-[:HAS_POLICY]-(bc:BoundedContext)
MATCH (pol)-[:INVOKES]->(cmd:Command)
WITH evt, collect({bc: bc.name, policy: pol.name, command: cmd.name}) as impacts
RETURN {
    event: evt.name,
    version: evt.version,
    affected_count: size(impacts),
    impacts: impacts
} as analysis
