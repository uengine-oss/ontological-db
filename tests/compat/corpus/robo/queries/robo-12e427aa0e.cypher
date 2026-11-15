MATCH (us:UserStory {id: $source_id})
MATCH (n {id: $target_id})
MERGE (us)-[:IMPLEMENTS]->(n)
RETURN us.id as id
