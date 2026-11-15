MATCH (p:Proposal {id: $pid})-[e:EFFECT]->(n)
RETURN n.id AS nodeId, labels(n) AS labels, e.diff AS diff, e.changeType AS ct
