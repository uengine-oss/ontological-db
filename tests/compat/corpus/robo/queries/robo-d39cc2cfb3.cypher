MATCH (pol:Policy {id: $source_id})
MATCH (cmd:Command {id: $target_id})
MERGE (pol)-[:INVOKES {isAsync: true}]->(cmd)
RETURN pol.id as id
