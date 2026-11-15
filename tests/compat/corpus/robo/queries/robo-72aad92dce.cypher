MATCH (agg:Aggregate {id: $id})
RETURN agg.key AS key, agg.invariants AS invariants,
       agg.invariantsMigratedAt AS migratedAt
