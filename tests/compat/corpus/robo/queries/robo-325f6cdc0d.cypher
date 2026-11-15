MATCH (cmd:Command {id: $cid}) OPTIONAL MATCH (cmd)-[:EMITS]->(evt:Event) RETURN cmd.actor AS actor, evt.sequence AS seq LIMIT 1
