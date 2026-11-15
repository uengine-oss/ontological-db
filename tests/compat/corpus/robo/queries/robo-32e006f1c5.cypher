MATCH (bc:BoundedContext {id: $bcId})
CREATE (evt:Event {
    id: randomUUID(),
    name: $name,
    displayName: $displayName,
    sequence: $sequence,
    actor: $actor,
    createdAt: datetime(),
    updatedAt: datetime()
})
MERGE (bc)-[:HAS_EVENT]->(evt)
RETURN evt.id AS id, evt.name AS name, evt.displayName AS displayName
