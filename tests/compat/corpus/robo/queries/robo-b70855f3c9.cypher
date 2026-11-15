MATCH (n:Aggregate) WHERE n.id = $id SET n.description = $value
