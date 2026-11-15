MATCH (evt:Event {id: $eventId})
OPTIONAL MATCH (oldBc:BoundedContext)-[oldRel:HAS_EVENT]->(evt)
DELETE oldRel
WITH evt
MATCH (newBc:BoundedContext {id: $targetBcId})
MERGE (newBc)-[:HAS_EVENT]->(evt)
