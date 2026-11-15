MATCH (pol:Policy {id: $node_id})
OPTIONAL MATCH (evt:Event)-[:TRIGGERS]->(pol)
OPTIONAL MATCH (pol)-[:INVOKES]->(cmd:Command)
OPTIONAL MATCH (cmd)-[:EMITS]->(target_evt:Event)
OPTIONAL MATCH (agg:Aggregate)-[:HAS_COMMAND]->(cmd)
RETURN evt, cmd, target_evt, agg
