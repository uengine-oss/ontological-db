MATCH (cmd:Command {id: $cid}), (evt:Event {id: $eid}) MERGE (cmd)-[r:EMITS]->(evt) ON CREATE SET r.createdAt = datetime()
