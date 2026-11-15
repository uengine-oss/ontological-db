MATCH (chg:RequirementChange {id: $change_id})
MATCH (n) WHERE n.id = $node_id
MERGE (chg)-[e:EFFECT]->(n)
ON CREATE SET e.reason = '직접 수정', e.impactLevel = 'HIGH'
ON MATCH SET e.reason = '직접 수정', e.impactLevel = 'HIGH'
