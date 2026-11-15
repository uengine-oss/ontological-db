MATCH (evt:Event {id: $node_id})-[:TRIGGERS]->(pol:Policy)
OPTIONAL MATCH (pol)-[:INVOKES]->(cmd:Command)
RETURN pol, cmd
