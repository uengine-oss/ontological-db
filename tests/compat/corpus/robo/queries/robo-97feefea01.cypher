MATCH ()-[e:EFFECT]->(ci:CreationIntent {id: $pid}) REMOVE e.appliedNodeId, e.diff, e.appliedAt
