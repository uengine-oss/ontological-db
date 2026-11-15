MATCH (bc:BoundedContext {id: $node_id})-[:HAS_POLICY]->(pol:Policy)
OPTIONAL MATCH (evt:Event)-[r:TRIGGERS]->(pol)
OPTIONAL MATCH (pol)-[r2:INVOKES]->(cmd:Command)
RETURN pol, evt.id as triggerEventId, cmd.id as invokeCommandId
