MATCH (e:Event {key: $ek, session_id: $sid}), (p:Policy {key: $pk, session_id: $sid}) MERGE (e)-[:TRIGGERS]->(p)
