MATCH (bc:BoundedContext {id: $bc_id})-[:HAS_AGGREGATE]->(agg:Aggregate)
OPTIONAL MATCH (agg)-[:HAS_COMMAND]->(cmd:Command)
OPTIONAL MATCH (cmd)-[:EMITS]->(evt:Event)
RETURN agg, cmd, evt
