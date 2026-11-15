MATCH (n:RequirementChange {id: $id})
OPTIONAL MATCH (n)-[e:EFFECT]->(t)
RETURN n {.*} AS change,
       collect({
           nodeId: t.id,
           nodeLabel: labels(t)[0],
           nodeTitle: COALESCE(t.title, t.name, t.action, ''),
           reason: e.reason,
           impactLevel: e.impactLevel
       }) AS effects
