MATCH (sourceEvt:Event)<-[:EMITS]-(sourceCmd:Command)<-[:HAS_COMMAND]-(sourceAgg:Aggregate)<-[:HAS_AGGREGATE]-(sourceBc:BoundedContext)
MATCH (sourceEvt)-[:TRIGGERS]->(pol:Policy)<-[:HAS_POLICY]-(targetBc:BoundedContext)
WHERE sourceBc <> targetBc
   AND sourceBc.id IN $bc_ids
   AND targetBc.id IN $bc_ids

OPTIONAL MATCH (pol)-[:INVOKES]->(targetCmd:Command)-[:EMITS]->(targetEvt:Event)

RETURN {
    sourceEventId: sourceEvt.id,
    sourceEventName: sourceEvt.name,
    sourceBcId: sourceBc.id,
    sourceBcName: sourceBc.name,

    policyId: pol.id,
    policyName: pol.name,

    targetCommandId: targetCmd.id,
    targetCommandName: targetCmd.name,
    targetEventId: targetEvt.id,
    targetEventName: targetEvt.name,
    targetBcId: targetBc.id,
    targetBcName: targetBc.name
} as connection
