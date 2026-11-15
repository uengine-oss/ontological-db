MATCH (bc:BoundedContext {id: $context_id})-[:HAS_AGGREGATE]->(agg:Aggregate)-[:HAS_INVARIANT]->(inv:Invariant)
OPTIONAL MATCH (inv)-[:VERIFIED_BY]->(c:Command)
OPTIONAL MATCH (inv)-[:HAS_GWT]->(g:GWT)
WITH agg.id AS aggregateId, inv,
     count(DISTINCT c) AS refCount, count(DISTINCT g) AS ownCount
RETURN aggregateId,
       inv {.id, .key, .name, .declaration, .source, .seq} AS invariant,
       refCount, ownCount
ORDER BY coalesce(inv.seq, 0), inv.declaration
