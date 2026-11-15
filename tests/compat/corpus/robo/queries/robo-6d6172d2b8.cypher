MATCH (a:Aggregate)
OPTIONAL MATCH (a)-[:HAS_COMMAND]->(c:Command)
OPTIONAL MATCH (c)-[:EMITS]->(ev:Event)
RETURN a.id AS id, coalesce(a.displayName, a.name) AS name,
       a.description AS description, a.stateTransitions AS st,
       a.invariants AS inv,
       collect(DISTINCT coalesce(c.displayName, c.name)) AS commands,
       collect(DISTINCT coalesce(ev.displayName, ev.name)) AS events
