MATCH (src:Command {id: $srcId}), (tgt:Event {id: $tgtId})
MERGE (tgt)-[:EMITS]->(src)
