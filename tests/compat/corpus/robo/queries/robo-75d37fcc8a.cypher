MATCH (p:Proposal {id: $id})
OPTIONAL MATCH (p)-[e:EFFECT]->(t)
RETURN p {.*} AS p,
       collect({
           nodeId: t.id,
           nodeLabel: labels(t)[0],
           nodeTitle: COALESCE(t.title, t.name, t.action, ''),
           reason: e.reason,
           impactLevel: e.impactLevel,
           changeType: e.changeType
       }) AS effects
