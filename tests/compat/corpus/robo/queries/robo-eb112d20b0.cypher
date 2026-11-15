MATCH (agg:Aggregate {id: $id}) RETURN agg.id AS id, agg.key AS key, agg.name AS name
