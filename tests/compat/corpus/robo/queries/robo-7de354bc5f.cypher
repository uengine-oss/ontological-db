MATCH (pol:Policy {id: $node_id})-[:INVOKES]->(cmd:Command)
RETURN cmd
