MATCH (agg:Aggregate {id: $agg})
CREATE (inv:Invariant {
    id: $id, declaration: $decl, name: $name, description: $desc,
    aggregateId: $agg, seq: $seq, source: 'ingested',
    proposalSource: $pid, createdAt: datetime(), updatedAt: datetime()
})
MERGE (agg)-[:HAS_INVARIANT]->(inv)
