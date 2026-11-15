MATCH (bc:BoundedContext)-[:HAS_EVENT]->(evt:Event)
RETURN bc.id AS bcId, bc.name AS bcName,
       evt.id AS evtId, evt.name AS evtName, evt.sequence AS evtSeq
