MERGE (cmd:Command {id: $cmd_id})
SET cmd.name = $name,
    cmd.actor = $actor,
    cmd.description = $description,
    cmd.createdAt = datetime()
WITH cmd
OPTIONAL MATCH (agg:Aggregate {id: $agg_id})
WHERE agg IS NOT NULL
MERGE (agg)-[:HAS_COMMAND]->(cmd)
RETURN cmd.id as id
