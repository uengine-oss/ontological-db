MATCH (agg:Aggregate {id: $aggregate_id})-[:HAS_COMMAND]->(cmd:Command)
OPTIONAL MATCH (cmd)-[:EMITS]->(evt:Event)
WITH cmd, collect(DISTINCT evt {.id, .name}) as emits
RETURN {
    id: cmd.id,
    name: cmd.name,
    displayName: cmd.displayName,
    actor: cmd.actor,
    category: cmd.category,
    inputSchema: cmd.inputSchema,
    description: cmd.description,
    emits: emits
} as command
ORDER BY cmd.name
