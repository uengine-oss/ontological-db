MATCH (agg:Aggregate) WHERE agg.id IN $ids RETURN agg.id AS id, agg.key AS key
