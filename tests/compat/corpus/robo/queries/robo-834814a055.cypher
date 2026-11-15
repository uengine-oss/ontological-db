MATCH (cmd1:Command)-[:EMITS]->(evt:Event)-[:TRIGGERS]->(pol:Policy)-[:INVOKES]->(cmd2:Command)
MATCH (bc1:BoundedContext)-[:HAS_AGGREGATE]->(agg1:Aggregate)-[:HAS_COMMAND]->(cmd1)
MATCH (bc2:BoundedContext)-[:HAS_POLICY]->(pol)
RETURN {
    source_bc: bc1.name,
    source_command: cmd1.name,
    event: evt.name,
    target_bc: bc2.name,
    policy: pol.name,
    target_command: cmd2.name
} as chain
