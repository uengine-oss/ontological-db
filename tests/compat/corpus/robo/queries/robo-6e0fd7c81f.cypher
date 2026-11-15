MATCH (cmd:Command {id: $cmd_id}) RETURN cmd.description as description
