MATCH (src:Command {id: $srcId}), (tgt:Event {id: $tgtId})
MERGE (src)-[:EMITS]->(tgt)
