MATCH (agg:Aggregate {id: $aid})
MERGE (inv:Invariant {key: $key})
  ON CREATE SET inv.id = randomUUID(),
                inv.createdAt = datetime(),
                inv.source = $source,
                inv.session_id = $session_id
SET inv.declaration = $declaration,
    inv.name = $name,
    inv.aggregateId = agg.id,
    inv.seq = coalesce(inv.seq, $seq),
    inv.updatedAt = datetime()
MERGE (agg)-[:HAS_INVARIANT]->(inv)
RETURN inv {.id, .key, .name, .declaration, .source, .seq} AS invariant
