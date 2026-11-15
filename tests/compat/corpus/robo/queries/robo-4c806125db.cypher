MATCH (evt:Event {id: $node_id})-[:TRIGGERS]->(pol:Policy)
OPTIONAL MATCH (pol)-[:INVOKES]->(cmd:Command)
OPTIONAL MATCH (polBc:BoundedContext)-[:HAS_POLICY]->(pol)
RETURN pol, cmd, polBc
