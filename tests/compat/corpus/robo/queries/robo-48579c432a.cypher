MATCH (bc:BoundedContext)-[:HAS_POLICY]->(pol:Policy)
OPTIONAL MATCH (evt:Event)-[:TRIGGERS]->(pol)
OPTIONAL MATCH (pol)-[:INVOKES]->(cmd:Command)
RETURN {
    id: pol.id,
    name: pol.name,
    description: pol.description,
    bcId: bc.id,
    triggerEventId: evt.id,
    invokeCommandId: cmd.id
} as policy
ORDER BY pol.name
