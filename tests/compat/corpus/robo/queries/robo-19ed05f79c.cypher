MATCH (agg:Aggregate {id: $aid})-[:HAS_INVARIANT]->(inv:Invariant)
OPTIONAL MATCH (inv)-[:VERIFIED_BY]->(c:Command)
OPTIONAL MATCH (inv)-[:HAS_GWT]->(g:GWT)
WITH inv, count(DISTINCT c) AS refCount, count(DISTINCT g) AS ownCount
RETURN inv {.id, .key, .name, .declaration, .description, .source, .seq} AS inv, refCount, ownCount
ORDER BY coalesce(inv.seq, 0), inv.declaration
