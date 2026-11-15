UNWIND $rows AS r
MATCH (agg:Aggregate {id: r.aggregate_id})
MERGE (cmd:Command {key: r.key})
  ON CREATE SET cmd.id = randomUUID(),
                cmd.createdAt = datetime()
SET cmd.key = r.key,
    cmd.name = r.name,
    cmd.displayName = r.display_name,
    cmd.actor = r.actor,
    cmd.category = r.category,
    cmd.inputSchema = r.input_schema,
    cmd.description = r.description,
    cmd.updatedAt = datetime()
MERGE (agg)-[:HAS_COMMAND]->(cmd)
RETURN cmd {.id, .key, .name, .displayName, .actor, .category, .inputSchema, .description} AS result
