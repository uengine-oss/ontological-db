MATCH (bc:BoundedContext)-[:HAS_EVENT]->(evt:Event)
RETURN bc.name AS bcName, evt.id AS evtId, evt.name AS evtName,
       evt.displayName AS evtDisplayName, evt.description AS evtDesc
ORDER BY bc.name, evt.name
