MATCH (p:Proposal {id: $pid})
MATCH (n:Aggregate {id: $nid})
MERGE (p)-[e:EFFECT]->(n)
SET e.reason = $reason, e.impactLevel = $impact,
    e.changeType = $ct, e.diff = $diff, e.appliedAt = datetime()
