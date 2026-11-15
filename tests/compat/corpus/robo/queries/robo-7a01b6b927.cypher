MATCH (bc:BoundedContext {id: $bc_id})
WHERE ($session_id IS NULL OR coalesce(bc.session_id, '') = $session_id)

// Step 1: Aggregate Properties and Additional Info
OPTIONAL MATCH (bc)-[:HAS_AGGREGATE]->(agg:Aggregate)
WITH bc, agg
OPTIONAL MATCH (agg)-[:HAS_PROPERTY]->(aggProp:Property)
WITH bc, agg, collect(DISTINCT aggProp {.id, .name, .displayName, .type, .isKey, .isForeignKey, .description, .fkTargetHint}) as aggProps

// Step 2: Commands with Properties and Additional Info
OPTIONAL MATCH (agg)-[:HAS_COMMAND]->(cmd:Command)
WITH bc, agg, aggProps, cmd
OPTIONAL MATCH (cmd)-[:HAS_PROPERTY]->(cmdProp:Property)
WITH bc, agg, aggProps, cmd, collect(DISTINCT cmdProp {.id, .name, .displayName, .type, .isRequired, .description}) as cmdProps
WITH bc, agg, aggProps, cmd, cmdProps,
     cmd.category as cmdCategory,
     cmd.inputSchema as cmdInputSchema,
     cmd.description as cmdDescription,
     cmd.displayName as cmdDisplayName

// Step 3: Events with Properties and Additional Info
OPTIONAL MATCH (cmd)-[:EMITS]->(evt:Event)
WITH bc, agg, aggProps, cmd, cmdProps, cmdCategory, cmdInputSchema, cmdDescription, cmdDisplayName, evt
OPTIONAL MATCH (evt)-[:HAS_PROPERTY]->(evtProp:Property)
WITH bc, agg, aggProps, cmd, cmdProps, cmdCategory, cmdInputSchema, cmdDescription, cmdDisplayName, evt, 
     collect(DISTINCT evtProp {.id, .name, .displayName, .type, .description}) as evtProps,
     evt.schema as evtSchema,
     evt.description as evtDescription,
     evt.displayName as evtDisplayName

// Step 4: Group Commands and Events by Aggregate
WITH bc, agg, aggProps,
     collect(DISTINCT {
         id: cmd.id, 
         name: cmd.name, 
         displayName: cmdDisplayName, 
         actor: cmd.actor, 
         category: cmdCategory,
         inputSchema: cmdInputSchema,
         description: cmdDescription,
         properties: cmdProps
     }) as commands,
     collect(DISTINCT {
         id: evt.id,
         name: evt.name,
         displayName: evtDisplayName,
         version: evt.version,
         schema: evtSchema,
         description: evtDescription,
         properties: evtProps,
         emittingCommandId: cmd.id,
         emittingCommandName: cmd.name
     }) as events
WITH bc, collect(DISTINCT {
    id: agg.id,
    name: agg.name,
    displayName: agg.displayName,
    rootEntity: agg.rootEntity,
    invariants: agg.invariants,
    enumerations: agg.enumerations,
    valueObjects: agg.valueObjects,
    properties: aggProps,
    commands: commands,
    events: events
}) as aggData

// Step 5: ReadModels with Properties and Additional Info
OPTIONAL MATCH (bc)-[:HAS_READMODEL]->(rm:ReadModel)
WITH bc, aggData, rm
OPTIONAL MATCH (rm)-[:HAS_PROPERTY]->(rmProp:Property)
WITH bc, aggData, rm, collect(DISTINCT rmProp {.id, .name, .displayName, .type, .description}) as rmProps
WITH bc, aggData, collect(DISTINCT {
    id: rm.id,
    name: rm.name,
    displayName: rm.displayName,
    description: rm.description,
    provisioningType: rm.provisioningType,
    actor: rm.actor,
    isMultipleResult: rm.isMultipleResult,
    properties: rmProps
}) as rmData

// Step 6: Policies with Cross-BC Information
OPTIONAL MATCH (bc)-[:HAS_POLICY]->(pol:Policy)
WITH bc, aggData, rmData, pol
OPTIONAL MATCH (triggerEvt:Event)-[:TRIGGERS]->(pol)
OPTIONAL MATCH (triggerEvt)<-[:EMITS]-(:Command)<-[:HAS_COMMAND]-(:Aggregate)<-[:HAS_AGGREGATE]-(triggerEvtBC:BoundedContext)
OPTIONAL MATCH (pol)-[:INVOKES]->(invokeCmd:Command)
OPTIONAL MATCH (invokeCmd)<-[:HAS_COMMAND]-(:Aggregate)<-[:HAS_AGGREGATE]-(invokeCmdBC:BoundedContext)
WITH bc, aggData, rmData, collect(DISTINCT {
    id: pol.id,
    name: pol.name,
    displayName: pol.displayName,
    description: pol.description,
    triggerEventId: triggerEvt.id,
    triggerEventName: triggerEvt.name,
    triggerEventBCId: triggerEvtBC.id,
    triggerEventBCName: triggerEvtBC.name,
    invokeCommandId: invokeCmd.id,
    invokeCommandName: invokeCmd.name,
    invokeCommandBCId: invokeCmdBC.id,
    invokeCommandBCName: invokeCmdBC.name
}) as polData

// Step 7: UI Wireframes
OPTIONAL MATCH (bc)-[:HAS_UI]->(ui:UI)
WITH bc, aggData, rmData, polData, collect(DISTINCT ui {.id, .name, .description, .template, .attachedToId, .attachedToType, .attachedToName}) as uiData

// Step 8: GWT
OPTIONAL MATCH (bc)-[:HAS_AGGREGATE]->(:Aggregate)-[:HAS_COMMAND]->(cmd2:Command)-[:HAS_GWT]->(gwt:GWT)
WITH bc, aggData, rmData, polData, uiData, collect(DISTINCT gwt {.id, .parentType, .parentId, .givenRef, .whenRef, .thenRef, .testCases}) as gwtData1
OPTIONAL MATCH (bc)-[:HAS_POLICY]->(pol2:Policy)-[:HAS_GWT]->(gwt2:GWT)
WITH bc, aggData, rmData, polData, uiData, gwtData1, collect(DISTINCT gwt2 {.id, .parentType, .parentId, .givenRef, .whenRef, .thenRef, .testCases}) as gwtData2
WITH bc, aggData, rmData, polData, uiData, gwtData1 + gwtData2 as gwtData

// Step 9: UserStories and their converted Analyzer rules.
OPTIONAL MATCH (us:UserStory)-[:IMPLEMENTS]->(bc)
WITH bc, aggData, rmData, polData, uiData, gwtData,
     collect(DISTINCT us) AS usList
UNWIND (CASE WHEN size(usList) = 0 THEN [null] ELSE usList END) AS us
OPTIONAL MATCH (us)-[:SOURCED_FROM]->(sr:Rule)
WITH bc, aggData, rmData, polData, uiData, gwtData, us,
     collect(DISTINCT CASE WHEN sr IS NOT NULL THEN
                          { rule_id: sr.id,
                            title: sr.title,
                            source_function: sr.source_function,
                            source_function_id: sr.source_function_id,
                            source_rule_id: sr.source_rule_id,
                            given: coalesce(sr.given, ''),
                            when_: coalesce(sr.`when`, ''),
                            then: coalesce(sr.`then`, ''),
                            write_effects_json: coalesce(sr.write_effects_json, '[]') }
                          ELSE NULL END) AS rawRules
WITH bc, aggData, rmData, polData, uiData, gwtData,
     collect(DISTINCT CASE WHEN us IS NOT NULL THEN {
         id: us.id,
         role: us.role,
         action: us.action,
         benefit: us.benefit,
         displayName: us.displayName,
         sourceUnitId: us.sourceUnitId,
         sourceRules: [r IN rawRules WHERE r IS NOT NULL]
     } ELSE NULL END) AS rawUsList
WITH bc, aggData, rmData, polData, uiData, gwtData,
     [u IN rawUsList WHERE u IS NOT NULL] AS userStoryData

RETURN {
    id: bc.id,
    name: bc.name,
    displayName: bc.displayName,
    description: bc.description,
    aggregates: [a IN aggData WHERE a.id IS NOT NULL],
    readmodels: [r IN rmData WHERE r.id IS NOT NULL],
    policies: [p IN polData WHERE p.id IS NOT NULL],
    uis: [u IN uiData WHERE u.id IS NOT NULL],
    gwts: [g IN gwtData WHERE g.id IS NOT NULL],
    userStories: [u IN userStoryData WHERE u.id IS NOT NULL]
} as bc_data
