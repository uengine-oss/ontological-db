MATCH (chg:RequirementChange {id: $cid})-[e:EFFECT]->(n) WHERE n.id = $nid REMOVE e.diff, e.appliedAt
