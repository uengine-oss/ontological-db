MATCH (cmd:Command) WHERE cmd.id IN $ids RETURN cmd.id AS id, cmd.actor AS actor
