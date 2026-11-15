MATCH (agg:Aggregate {id: $agg_id}), (cmd:Command {id: $cmd_id}) MERGE (agg)-[:HAS_COMMAND]->(cmd)
