MATCH (agg:Aggregate {id: $aggregate_id})
MERGE (cmd:Command {key: $key})
ON CREATE SET cmd.id = randomUUID(),
              cmd.createdAt = datetime()
SET cmd.key = $key,
    cmd.name = $name,
    cmd.displayName = $display_name,
    cmd.actor = $actor,
    cmd.category = $category,
    cmd.inputSchema = $input_schema,
    cmd.description = $description,
    cmd.updatedAt = datetime()
MERGE (agg)-[:HAS_COMMAND]->(cmd)
RETURN cmd {.id, .key, .name, .displayName, .actor, .category, .inputSchema, .description} as command
