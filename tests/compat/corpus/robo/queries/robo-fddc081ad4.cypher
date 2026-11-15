MATCH (cmd:Command {id: $cmd_id})-[:HAS_PROPERTY]->(p:Property) RETURN p.name as name, p.type as type ORDER BY p.name
