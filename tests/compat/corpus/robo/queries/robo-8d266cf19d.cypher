MATCH (bc:BoundedContext)-[:HAS_READMODEL]->(rm:ReadModel)
OPTIONAL MATCH (rmUi:UI)-[:ATTACHED_TO]->(rm)
OPTIONAL MATCH (rm)-[:HAS_CQRS]->(:CQRSConfig)-[:HAS_OPERATION]->(op:CQRSOperation)-[:TRIGGERED_BY]->(triggerEvt:Event)
WITH rm, bc, rmUi, collect(DISTINCT triggerEvt.id) AS triggerEventIds
RETURN rm.id AS rmId, rm.name AS rmName, rm.displayName AS rmDisplayName,
       rm.actor AS rmActor, rm.sequence AS rmStoredSequence,
       bc.id AS bcId, bc.name AS bcName,
       rmUi.id AS rmUiId, rmUi.name AS rmUiName,
       rmUi.displayName AS rmUiDisplayName, rmUi.template AS rmUiTemplate,
       triggerEventIds
