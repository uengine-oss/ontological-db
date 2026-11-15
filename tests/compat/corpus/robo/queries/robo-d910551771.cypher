MATCH (bc:BoundedContext)-[:HAS_EVENT]->(evt:Event)
WHERE NOT (()-[:EMITS]->(evt))
RETURN DISTINCT bc.id AS bcId, bc.name AS bcName, bc.displayName AS bcDisplayName,
       evt.id AS evtId, evt.name AS evtName, evt.displayName AS evtDisplayName,
       evt.sequence AS evtTimelineSequence
