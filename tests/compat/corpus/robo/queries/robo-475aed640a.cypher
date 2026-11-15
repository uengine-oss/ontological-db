MATCH (us:UserStory)-[r:IMPLEMENTS]->(old_agg:Aggregate {id: $old_id})
MATCH (new_agg:Aggregate {id: $new_id})
MERGE (us)-[:IMPLEMENTS]->(new_agg)
DELETE r
