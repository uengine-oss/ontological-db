MATCH (p:Proposal {id: $id})-[:EFFECT]->(n)
MATCH (other:Proposal)-[:EFFECT]->(n)
WHERE other.id <> $id AND other.status = 'IMPLEMENTING'
RETURN DISTINCT other.id AS conflictId
