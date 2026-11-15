MATCH (us:UserStory {id: $id})-[:IMPLEMENTS]->(cmd:Command)
RETURN cmd {.*} AS cmd LIMIT 1
