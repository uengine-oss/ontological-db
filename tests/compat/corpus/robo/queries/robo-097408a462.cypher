MATCH (x {id: $id})-[:READS|WRITES]->(t:TABLE) RETURN DISTINCT t.id AS id, t.name AS name
