MATCH (us:UserStory {id: $us_id})
MATCH (bc:BoundedContext {id: $bc_id})
MERGE (us)-[:IMPLEMENTS]->(bc)
RETURN bc.id as id
