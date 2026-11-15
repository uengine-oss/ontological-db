MATCH (agg:Aggregate {id: $id})
OPTIONAL MATCH (agg)-[:HAS_COMMAND]->(cmd:Command)
OPTIONAL MATCH (cmd)-[:EMITS]->(evt:Event)
OPTIONAL MATCH (cmd)-[:HAS_PROPERTY]->(cmd_prop:Property)
OPTIONAL MATCH (evt)-[:HAS_PROPERTY]->(evt_prop:Property)
OPTIONAL MATCH (agg)-[:HAS_PROPERTY]->(agg_prop:Property)
DETACH DELETE evt_prop, cmd_prop, agg_prop, evt, cmd, agg
