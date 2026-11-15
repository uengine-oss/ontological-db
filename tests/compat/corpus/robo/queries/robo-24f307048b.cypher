MATCH (us:UserStory {id: $us_id}), (f:Feature {id: $fid}) MERGE (f)-[:CONTAINS]->(us)
