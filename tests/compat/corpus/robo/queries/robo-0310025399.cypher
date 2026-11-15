MERGE (bc:BoundedContext {id: $bc_id})
SET bc.name = $name,
    bc.description = $description,
    bc.createdAt = datetime()
WITH bc
MATCH (us:UserStory {id: $us_id})
MERGE (us)-[:IMPLEMENTS]->(bc)
RETURN bc.id as id
