MATCH (a:Aggregate {id:$id}) RETURN coalesce(a.version,0) AS v
