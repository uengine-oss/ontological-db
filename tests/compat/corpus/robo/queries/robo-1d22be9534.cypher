MATCH (bc:BoundedContext {id: $bc_id})
OPTIONAL MATCH (bc)-[:HAS_AGGREGATE]->(agg:Aggregate)
OPTIONAL MATCH (agg)-[:HAS_COMMAND]->(cmd:Command)
OPTIONAL MATCH (cmd)-[:EMITS]->(evt:Event)
RETURN
    collect(DISTINCT {id: agg.id, name: agg.name, type: 'Aggregate'}) as aggregates,
    collect(DISTINCT {id: cmd.id, name: cmd.name, type: 'Command'}) as commands,
    collect(DISTINCT {id: evt.id, name: evt.name, type: 'Event'}) as events
