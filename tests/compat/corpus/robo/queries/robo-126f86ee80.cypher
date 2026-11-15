MATCH (pol:Policy {id: $source_id})
MATCH (cmd:Command {id: $target_id})
MERGE (pol)-[:INVOKES]->(cmd)
RETURN pol.id as id
