MATCH (chg:RequirementChange {id: $id})
OPTIONAL MATCH (chg)-[e:EFFECT]->(n)
RETURN chg.title AS title,
       chg.originalPrompt AS originalPrompt,
       collect({
           nodeId: n.id,
           nodeLabel: labels(n)[0],
           nodeTitle: COALESCE(n.title, n.name, n.action, ''),
           reason: e.reason,
           impactLevel: e.impactLevel
       }) AS effects
