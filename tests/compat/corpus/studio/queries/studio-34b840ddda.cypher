MATCH (n:_Entity:Customer) DETACH DELETE n RETURN count(n) AS cnt
