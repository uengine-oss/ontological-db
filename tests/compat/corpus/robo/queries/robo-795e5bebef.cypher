MATCH (n:Aggregate) WHERE n.id = $ref OR n.key = $ref RETURN n.id AS id LIMIT 1
