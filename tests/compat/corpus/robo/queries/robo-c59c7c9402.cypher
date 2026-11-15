MATCH (agg:Aggregate {id: $aid})
MERGE (inv:Invariant {key: $key})
  ON CREATE SET inv.id = randomUUID(), inv.createdAt = datetime()
SET inv.declaration = $declaration,
    inv.name = $name,
    inv.source = 'migrated',
    inv.seq = $seq,
    inv.aggregateId = agg.id,
    inv.updatedAt = datetime()
MERGE (agg)-[:HAS_INVARIANT]->(inv)
