MATCH (cs:ChangeSet {id: $id})-[:CONTAINS]->(chg) RETURN chg.author AS author LIMIT 1
