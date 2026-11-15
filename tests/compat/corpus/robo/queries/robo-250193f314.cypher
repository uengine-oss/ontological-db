MATCH (a:Aggregate {id: $id})
OPTIONAL MATCH (a)-[:HAS_COMMAND]->(c:Command)
OPTIONAL MATCH (c)-[:EMITS]->(e:Event)
OPTIONAL MATCH (a)-[:HAS_INVARIANT]->(i:Invariant)
RETURN coalesce(a.displayName, a.name) AS name, a.description AS description,
       a.stateTransitions AS stateTransitions, a.correctivePolicies AS correctivePolicies,
       a.throughput AS throughput, a.invariants AS invProp,
       coalesce(a.version, 0) AS version,
       collect(DISTINCT coalesce(c.displayName, c.name)) AS commands,
       collect(DISTINCT coalesce(e.displayName, e.name)) AS events,
       collect(DISTINCT coalesce(i.name, i.description)) AS invNodes
