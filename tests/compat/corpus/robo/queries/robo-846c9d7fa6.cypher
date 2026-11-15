MATCH (us:UserStory)-[r:IMPLEMENTS]->(old_cmd:Command {id: $old_id})
MATCH (new_cmd:Command {id: $new_id})
MERGE (us)-[:IMPLEMENTS]->(new_cmd)
DELETE r
