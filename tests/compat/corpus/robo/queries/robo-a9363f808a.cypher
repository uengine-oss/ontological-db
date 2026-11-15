MATCH (us:UserStory {id: $us_id}), (bc:BoundedContext {id: $bc_id}) MERGE (bc)-[:CONTAINS_STORY]->(us)
