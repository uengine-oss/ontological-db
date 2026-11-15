MATCH (cmd:Command)-[:EMITS]->(evt:Event)
RETURN cmd.id AS cmdId, evt {.id, .name, .displayName, .description} AS evt
