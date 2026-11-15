MATCH (cmd:Command {id: $command_id})
MERGE (evt:Event {key: $key})
ON CREATE SET evt.id = randomUUID(),
              evt.createdAt = datetime()
SET evt.key = $key,
    evt.name = $name,
    evt.displayName = $display_name,
    evt.version = $version,
    evt.schema = $schema,
    evt.payload = $payload,
    evt.description = $description,
    evt.isBreaking = false,
    evt.updatedAt = datetime()
MERGE (cmd)-[:EMITS {isGuaranteed: true}]->(evt)
RETURN evt {.id, .key, .name, .displayName, .version, .schema, .payload, .description} as event
