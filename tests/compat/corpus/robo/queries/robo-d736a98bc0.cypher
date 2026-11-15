MATCH (agg:Aggregate {id: $agg_id}) RETURN agg.enumerations as enumerations, agg.valueObjects as valueObjects
