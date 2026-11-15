MATCH (bc:BoundedContext {id: $node_id})-[r:HAS_AGGREGATE]->(agg:Aggregate)
OPTIONAL MATCH (agg)-[r2:HAS_COMMAND]->(cmd:Command)
OPTIONAL MATCH (cmd)-[r3:EMITS]->(evt:Event)
RETURN agg, cmd, evt,
       {source: bc.id, target: agg.id, type: 'HAS_AGGREGATE'} as rel1,
       {source: agg.id, target: cmd.id, type: 'HAS_COMMAND'} as rel2,
       {source: cmd.id, target: evt.id, type: 'EMITS'} as rel3
