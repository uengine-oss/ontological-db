MATCH (agg:Aggregate {id: $aggId})
CREATE (cmd:Command {
    id: randomUUID(),
    name: $name,
    displayName: $displayName,
    sequence: $sequence,
    actor: $actor,
    createdAt: datetime(),
    updatedAt: datetime()
})
MERGE (agg)-[:HAS_COMMAND]->(cmd)
RETURN cmd.id AS id, cmd.name AS name, cmd.displayName AS displayName
