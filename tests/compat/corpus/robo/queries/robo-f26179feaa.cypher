MATCH (cmd:Command {id: $id})
MATCH (agg:Aggregate {id: $agg_id})
MERGE (agg)-[:HAS_COMMAND]->(cmd)
