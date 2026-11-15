MATCH (pol:Policy)-[:INVOKES]->(cmd:Command)
WHERE NOT (:Aggregate)-[:HAS_COMMAND]->(cmd)
MATCH (bc:BoundedContext)-[:HAS_POLICY]->(pol)
OPTIONAL MATCH (bc)-[:HAS_AGGREGATE]->(agg:Aggregate)
RETURN cmd.id AS cmdId, cmd.name AS cmdName,
       bc.id AS bcId, bc.name AS bcName,
       collect(agg.id)[0] AS firstAggId
