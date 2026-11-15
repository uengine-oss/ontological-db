MATCH (cmd:Command {id: $id})
OPTIONAL MATCH (cmd)-[:EMITS]->(evt:Event)
OPTIONAL MATCH (cmd)-[:HAS_PROPERTY]->(cmd_prop:Property)
OPTIONAL MATCH (evt)-[:HAS_PROPERTY]->(evt_prop:Property)
DETACH DELETE evt_prop, cmd_prop, evt, cmd
