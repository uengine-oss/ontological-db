MERGE (cmd:Command {id: $cmd_id})
SET cmd.name = $name,
    cmd.description = $description,
    cmd.createdAt = datetime()
RETURN cmd.id as id
