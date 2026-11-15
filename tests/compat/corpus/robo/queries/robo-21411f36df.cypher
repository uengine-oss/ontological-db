MATCH (agg:Aggregate {id: $id})
SET agg.invariantsMigratedAt = datetime(), agg.invariants = []
