MATCH (agg:Aggregate {id: $node_id})-[:HAS_COMMAND]->(cmd:Command)
OPTIONAL MATCH (cmd)-[:EMITS]->(evt:Event)
RETURN cmd, evt
