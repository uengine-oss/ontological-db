MATCH (bc:BoundedContext {id: $bcId})
CREATE (rm:ReadModel {
    id: randomUUID(),
    name: $name,
    displayName: $displayName,
    sequence: $sequence,
    actor: $actor,
    createdAt: datetime(),
    updatedAt: datetime()
})
MERGE (bc)-[:HAS_READMODEL]->(rm)
RETURN rm.id AS id, rm.name AS name, rm.displayName AS displayName
