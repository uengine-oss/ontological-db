MATCH (tgt:Event {id: $tgtId})-[r:EMITS]->(src:Command {id: $srcId})
DELETE r
