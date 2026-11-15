MATCH (evt:Event)
OPTIONAL MATCH (bc:BoundedContext)-[:HAS_EVENT]->(evt)
OPTIONAL MATCH (us:UserStory)-[:HAS_EVENT]->(evt)
OPTIONAL MATCH (us)-[:IMPLEMENTS]->(us_bc:BoundedContext)
RETURN evt.id AS evtId, evt.name AS evtName,
       evt.displayName AS evtDisplayName, evt.description AS evtDesc,
       COALESCE(bc.name, us_bc.name, 'Unknown') AS bcName
ORDER BY bcName, evt.name
