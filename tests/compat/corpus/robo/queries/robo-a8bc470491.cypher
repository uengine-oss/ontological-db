MATCH (sourceEvt:Event)<-[:EMITS]-(sourceCmd:Command)<-[:HAS_COMMAND]-(sourceAgg:Aggregate)<-[:HAS_AGGREGATE]-(bc:BoundedContext)
MATCH (sourceEvt)-[:TRIGGERS]->(pol:Policy)<-[:HAS_POLICY]-(bc)
WHERE bc.id IN $bc_ids

OPTIONAL MATCH (pol)-[:INVOKES]->(targetCmd:Command)-[:EMITS]->(targetEvt:Event)
WHERE targetEvt <> sourceEvt

RETURN {
    sourceEventId: sourceEvt.id,
    sourceEventName: sourceEvt.name,
    bcId: bc.id,
    bcName: bc.name,

    policyId: pol.id,
    policyName: pol.name,

    targetCommandId: targetCmd.id,
    targetCommandName: targetCmd.name,
    targetEventId: targetEvt.id,
    targetEventName: targetEvt.name,
    isSameBC: true
} as connection
