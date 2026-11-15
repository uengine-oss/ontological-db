MATCH (chg:RequirementChange {id: $cid})-[e:EFFECT]->(ci:CreationIntent {id: $pid}) SET e.appliedNodeId = $real_id, e.appliedAt = $now
