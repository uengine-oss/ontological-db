MATCH (n:Aggregate) WHERE n.id = $id RETURN n.description AS v
