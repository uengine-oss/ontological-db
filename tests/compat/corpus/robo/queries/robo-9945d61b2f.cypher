MATCH (bc:BoundedContext)-[:HAS_AGGREGATE]->(agg:Aggregate)-[:HAS_COMMAND]->(cmd:Command)
OPTIONAL MATCH (cmd)-[:EMITS]->(evt:Event)
OPTIONAL MATCH (cmdUi:UI)-[:ATTACHED_TO]->(cmd)
RETURN cmd.id AS cmdId, cmd.name AS cmdName, cmd.displayName AS cmdDisplayName,
       cmd.actor AS cmdActor, cmd.sequence AS cmdStoredSequence,
       agg.id AS aggId, agg.name AS aggName,
       bc.id AS bcId, bc.name AS bcName, bc.displayName AS bcDisplayName,
       evt.id AS evtId, evt.name AS evtName, evt.displayName AS evtDisplayName,
       evt.sequence AS evtTimelineSequence,
       cmdUi.id AS cmdUiId, cmdUi.name AS cmdUiName,
       cmdUi.displayName AS cmdUiDisplayName, cmdUi.template AS cmdUiTemplate,
       cmdUi.description AS cmdUiDescription
