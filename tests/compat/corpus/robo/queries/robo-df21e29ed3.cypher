UNWIND $rows AS r
MATCH (cmd:Command {id: r.command_id})
MERGE (evt:Event {key: r.key})
  ON CREATE SET evt.id = randomUUID(),
                evt.createdAt = datetime()
SET evt.key = r.key,
    evt.name = r.name,
    evt.displayName = r.display_name,
    evt.version = r.version,
    evt.schema = r.schema,
    evt.payload = r.payload,
    evt.description = r.description,
    evt.isBreaking = false,
    evt.updatedAt = datetime()
MERGE (cmd)-[:EMITS {isGuaranteed: true}]->(evt)
RETURN evt {.id, .key, .name, .displayName, .version, .schema, .payload, .description} AS result
