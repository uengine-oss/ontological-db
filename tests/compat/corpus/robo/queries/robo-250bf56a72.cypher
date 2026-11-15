MATCH (agg:Aggregate)-[:HAS_COMMAND]->(cmd:Command {id: $cmd_id})
RETURN agg {.*} AS agg
