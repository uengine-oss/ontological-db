MATCH (x {id: $id})-[:HAS_RULE]->(:RULE)-[:READS|WRITES]->(t:TABLE) RETURN DISTINCT t.id AS id, t.name AS name
