MATCH (bc:BoundedContext)-[:HAS_AGGREGATE]->(agg:Aggregate)-[:HAS_COMMAND]->(cmd:Command)-[:EMITS]->(evt:Event)
OPTIONAL MATCH (evt)-[:HAS_PROPERTY]->(prop:Property)
WITH bc, evt, collect(DISTINCT prop {.id, .name, .type}) as properties
RETURN {
    id: evt.id,
    name: evt.name,
    bcId: bc.id,
    bcName: bc.name,
    properties: properties
} as event
ORDER BY bc.name, evt.name
