MATCH (bc:BoundedContext)
OPTIONAL MATCH (bc)-[:HAS_AGGREGATE]->(agg:Aggregate)
OPTIONAL MATCH (agg)-[:HAS_COMMAND]->(cmd:Command)
OPTIONAL MATCH (cmd)-[:EMITS]->(evt:Event)
OPTIONAL MATCH (bc)-[:HAS_POLICY]->(pol:Policy)

WITH bc,
     collect(DISTINCT agg {.id, .name, .rootEntity}) as aggregates,
     collect(DISTINCT cmd {.id, .name, .actor}) as commands,
     collect(DISTINCT evt {.id, .name, .version}) as events,
     collect(DISTINCT pol {.id, .name, .triggerCondition}) as policies

RETURN bc {.id, .name, .description,
    aggregates: aggregates,
    commands: commands,
    events: events,
    policies: policies
} as boundedContext
