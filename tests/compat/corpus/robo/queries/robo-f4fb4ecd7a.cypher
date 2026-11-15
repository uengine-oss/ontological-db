MATCH (chg:RequirementChange {id: $change_id})
MATCH (n) WHERE n.id = $node_id
MERGE (chg)-[e:EFFECT]->(n)
ON CREATE SET e.reason = $reason, e.impactLevel = $impact_level, e.changeType = 'MODIFY'
ON MATCH SET e.reason = $reason, e.impactLevel = $impact_level, e.changeType = 'MODIFY'
