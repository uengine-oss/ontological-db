MATCH (agg:Aggregate {id: $id})
SET agg.exceptions = $exceptions, agg.updatedAt = datetime()
RETURN agg.id AS id
