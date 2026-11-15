MATCH (n:_Entity:Customer) WHERE n.grade = $grade RETURN n.name AS name, n.grade AS grade, n.credit_limit AS credit_limit
