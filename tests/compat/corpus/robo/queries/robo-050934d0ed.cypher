MATCH (jn:Journey {id: $jid})
CREATE (s:JourneyStep {id: $id, name: $name, kind: $kind, seq: $seq,
    proposalSource: $pid, createdAt: datetime()})
MERGE (jn)-[:HAS_STEP]->(s)
