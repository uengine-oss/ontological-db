MATCH (cmd:Command {id: $cmd_id}), (evt:Event {id: $evt_id}) MERGE (cmd)-[r:EMITS]->(evt) ON CREATE SET r.isGuaranteed = true
