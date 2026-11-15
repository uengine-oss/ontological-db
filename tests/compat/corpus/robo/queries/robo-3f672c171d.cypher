MATCH (cmd:Command {id: $cmd_id})-[:EMITS]->(evt:Event)
RETURN evt {.id, .key, .name, .displayName, .version, .schema, .payload} as event
ORDER BY evt.name
