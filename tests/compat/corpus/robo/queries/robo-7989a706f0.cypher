MATCH (cmd:Command {id: $cmd_id})-[:EMITS]->(evt:Event)
OPTIONAL MATCH (evt)-[:TRIGGERS]->(pol:Policy)
OPTIONAL MATCH (pol)-[:INVOKES]->(next:Command)
RETURN evt {.*} AS evt, pol {.*} AS pol, next {.*} AS next
