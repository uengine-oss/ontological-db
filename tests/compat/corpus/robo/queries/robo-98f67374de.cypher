MATCH (bc:BoundedContext)-[:HAS_AGGREGATE]->(:Aggregate)-[:HAS_COMMAND]->(cmd:Command)
WHERE NOT (cmd)-[:EMITS]->(:Event)
RETURN cmd.name AS cmdName, bc.name AS bcName
ORDER BY bc.name, cmd.name
