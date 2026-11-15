MATCH (src:Command {id: $srcId})-[r:EMITS]->(tgt:Event {id: $tgtId})
DELETE r
