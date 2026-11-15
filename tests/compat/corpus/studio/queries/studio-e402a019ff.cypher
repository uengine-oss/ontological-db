MATCH (n:_Entity:Customer)
WHERE n.name CONTAINS $name
RETURN n
LIMIT 10
