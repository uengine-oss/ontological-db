MATCH (p:Policy {key: $pk, session_id: $sid}), (c:Command {key: $ck, session_id: $sid}) MERGE (p)-[:INVOKES]->(c)
