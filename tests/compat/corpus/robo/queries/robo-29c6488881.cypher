MATCH (bc:BoundedContext {id: $context_id})-[:HAS_POLICY]->(pol:Policy)
OPTIONAL MATCH (evt:Event)-[:TRIGGERS]->(pol)
OPTIONAL MATCH (pol)-[:INVOKES]->(cmd:Command)
RETURN pol {.id, .name, .displayName, .description} as policy,
       evt.id as triggerEventId,
       cmd.id as invokeCommandId
ORDER BY pol.name
