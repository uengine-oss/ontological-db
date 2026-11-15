MATCH (agg {id: $aid})
WHERE 'Aggregate' IN labels(agg) OR 'Command' IN labels(agg)
   OR 'Event' IN labels(agg) OR 'ReadModel' IN labels(agg)
MERGE (prop:Property {parentType: head(labels(agg)), parentId: $aid, name: $name})
ON CREATE SET prop.id = randomUUID(), prop.createdAt = datetime()
SET prop.type = $type, prop.parentType = head(labels(agg)),
    prop.parentId = $aid, prop.updatedAt = datetime()
MERGE (agg)-[:HAS_PROPERTY]->(prop)
