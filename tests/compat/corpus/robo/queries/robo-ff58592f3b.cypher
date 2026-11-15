MATCH (bc:BoundedContext {id: $bc_id})-[:HAS_POLICY]->(pol:Policy)
OPTIONAL MATCH (evt:Event)-[:TRIGGERS]->(pol)
OPTIONAL MATCH (pol)-[:INVOKES]->(cmd:Command)
RETURN pol, evt.id as triggerEventId, cmd.id as invokeCommandId
