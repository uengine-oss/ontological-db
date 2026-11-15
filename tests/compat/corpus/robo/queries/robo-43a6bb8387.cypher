MATCH (cs:ChangeSet {id: $id})-[:CONTAINS]->(chg) WHERE chg.status = $s RETURN chg.id AS id, chg.statusHistory AS hist
