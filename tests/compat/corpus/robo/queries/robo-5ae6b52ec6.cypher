MATCH (bc:BoundedContext {id: $context_id})
OPTIONAL MATCH (bc)-[:HAS_AGGREGATE]->(agg:Aggregate)
OPTIONAL MATCH (agg)-[:HAS_COMMAND]->(cmd:Command)
OPTIONAL MATCH (cmd)-[:EMITS]->(evt:Event)
OPTIONAL MATCH (bc)-[:HAS_POLICY]->(pol:Policy)
WITH bc, agg, cmd, evt, pol
WITH bc,
     agg,
     collect(DISTINCT {
         id: cmd.id,
         name: cmd.name,
         type: 'Command',
         actor: cmd.actor
     }) as commands,
     collect(DISTINCT {
         id: evt.id,
         name: evt.name,
         type: 'Event',
         version: evt.version
     }) as events
WITH bc,
     collect(DISTINCT {
         id: agg.id,
         name: agg.name,
         type: 'Aggregate',
         rootEntity: agg.rootEntity,
         commands: commands,
         events: events
     }) as aggregates
MATCH (bc)-[:HAS_POLICY]->(pol:Policy)
OPTIONAL MATCH (evt:Event)-[:TRIGGERS]->(pol)
OPTIONAL MATCH (pol)-[:INVOKES]->(cmd:Command)
WITH bc, aggregates,
     collect(DISTINCT {
         id: pol.id,
         name: pol.name,
         type: 'Policy',
         description: pol.description,
         triggerEventId: evt.id,
         invokeCommandId: cmd.id
     }) as policies
RETURN {
    id: bc.id,
    name: bc.name,
    type: 'BoundedContext',
    description: bc.description,
    classification: bc.classification,
    aggregates: aggregates,
    policies: policies
} as tree
