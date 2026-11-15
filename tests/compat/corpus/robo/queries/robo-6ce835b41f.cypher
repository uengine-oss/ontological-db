MATCH (us:UserStory {id: $source_id})
MATCH (n {id: $target_id})
MERGE (us)-[:IMPLEMENTS {createdAt: datetime()}]->(n)
RETURN us.id as id
