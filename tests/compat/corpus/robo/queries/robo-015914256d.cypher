MATCH (cmd:Command {id: $cmd_id})-[:EMITS]->(evt:Event)
RETURN evt {
    .id, .key, .name, .displayName, .version, .schema, .payload, .description, .sequence
} AS event
ORDER BY evt.name
