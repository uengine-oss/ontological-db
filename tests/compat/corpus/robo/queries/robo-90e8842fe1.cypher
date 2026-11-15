MATCH (chg:RequirementChange {id: $id})-[e:EFFECT]->(n)
RETURN n.id AS nodeId,
       labels(n)[0] AS nodeLabel,
       COALESCE(n.title, n.name, n.action, '') AS nodeTitle,
       e.reason AS reason,
       e.impactLevel AS impactLevel
