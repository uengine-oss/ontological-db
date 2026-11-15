MATCH (n:_Entity) WHERE elementId(n) = $eid SET n.embedding = $vec
