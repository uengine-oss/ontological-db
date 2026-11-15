MATCH (agg:Aggregate {id: $aid})
MERGE (inv:Invariant {key: $key})
  ON CREATE SET inv.id = randomUUID(), inv.createdAt = datetime()
SET inv.declaration = $declaration,
    inv.name = $name,
    inv.description = $description,
    inv.source = $source,
    inv.seq = $seq,
    inv.aggregateId = agg.id,
    inv.updatedAt = datetime()
MERGE (agg)-[:HAS_INVARIANT]->(inv)
RETURN inv {.id, .key, .name, .declaration, .description, .source, .seq} AS inv
