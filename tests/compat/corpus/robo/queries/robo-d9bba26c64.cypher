MATCH (chg:RequirementChange {id: $cid})-[e:EFFECT]->(n) WHERE n.id = $nid SET e.diff = $diff, e.appliedAt = $at
