MERGE (n:Command {id: $target_id})
SET n.name = $name, n.description = $description, n.createdAt = datetime()
WITH n
MATCH (agg:Aggregate {id: $agg_id})
MERGE (agg)-[:HAS_COMMAND]->(n)
RETURN n.id as id
