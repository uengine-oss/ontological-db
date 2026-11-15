MATCH (bc:BoundedContext {id: $bc_id})-[:HAS_POLICY]->(pol:Policy)
OPTIONAL MATCH (triggerEvt:Event)-[:TRIGGERS]->(pol)
OPTIONAL MATCH (pol)-[:INVOKES]->(invokedCmd:Command)
RETURN pol, triggerEvt.id as triggerEventId, invokedCmd.id as invokeCommandId
