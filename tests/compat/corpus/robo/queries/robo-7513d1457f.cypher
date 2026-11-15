MATCH (agg:Aggregate)-[:HAS_INVARIANT]->(inv:Invariant {id: $id})
MATCH (agg)-[:HAS_COMMAND]->(cmd:Command)
OPTIONAL MATCH (cmd)-[:HAS_GWT]->(g:GWT)
RETURN cmd.id AS commandId, cmd.name AS commandName,
       (g IS NOT NULL) AS hasGwt,
       EXISTS { (inv)-[:VERIFIED_BY]->(cmd) } AS alreadyReferenced
ORDER BY cmd.name
